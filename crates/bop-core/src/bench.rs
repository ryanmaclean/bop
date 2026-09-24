//! Common BOP state-machine benchmark workload (ryanmaclean/bop#8).
//!
//! Drives the filesystem-as-state-machine through a fixed, seed-derived
//! sequence and emits one `ryanlab.bench.v1` record (schema:
//! ryanmaclean/skills `schemas/bench.v1.schema.json`, contract:
//! `docs/BENCHMARKING.md`).
//!
//! Per card, in this order:
//!
//! ```text
//! CREATE -> CLAIM (pending->running) -> APPEND logs -> WRITE artifact ->
//! COMPLETE | FAIL -> FSYNC -> CRASH -> RECOVER -> QUERY HISTORY -> EXPORT LINEAGE
//! ```
//!
//! - The core uses only `mkdir`, `rename`, `write`, `fsync` and `read`. There
//!   is no filesystem-specific logic here. Filesystem-native version ids
//!   (HAMMER TID, LFS checkpoint, FFS snapshot) come from a [`VersionProbe`]
//!   adapter. The default [`NoVersion`] probe reports none.
//! - CRASH appends a torn partial frame after the last committed transition.
//!   RECOVER must report the torn tail and return exactly the committed
//!   records. The next transition (`done -> merged`, or the operator retry
//!   `failed -> pending`) truncates the tail and bumps the epoch (C10).
//! - Replay oracle: every committed record is also kept in memory. After the
//!   whole tree has been written, each log is re-read with
//!   [`translog::recover_bytes`] and projected. The projection must equal the
//!   in-memory projection and the state directory the card sits in.
//! - `replay_digest` is blake3 over the canonical record bodies of every card
//!   (card order) plus each final state. It has no clocks and no paths, so the
//!   same config gives the same digest on every backend.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::translog::{self, CardState, Commit, LineageStep, Proposal, Record};

/// Output schema (ryanlab shared benchmark contract).
pub const BENCH_SCHEMA: &str = "ryanlab.bench.v1";
/// Stable workload name. Config knobs are reported in `tags`.
pub const WORKLOAD: &str = "bop.state-machine.v1";
/// Workload steps, in execution order.
pub const STEPS: [&str; 10] = [
    "create",
    "claim",
    "append_logs",
    "write_artifact",
    "complete_or_fail",
    "fsync",
    "crash",
    "recover",
    "query_history",
    "export_lineage",
];

/// Workload configuration. Everything except `runtime`, `filesystem` and
/// `commit` changes the replay digest.
#[derive(Debug, Clone)]
pub struct Config {
    pub cards: usize,
    pub seed: u64,
    pub artifact_bytes: usize,
    pub log_lines: usize,
    pub project: String,
    pub runtime: Option<String>,
    pub filesystem: Option<String>,
    pub commit: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            cards: 16,
            seed: 1,
            artifact_bytes: 4096,
            log_lines: 32,
            project: "bop".into(),
            runtime: None,
            filesystem: None,
            commit: None,
        }
    }
}

/// Backend adapter for filesystem-native version identity (bop#7).
pub trait VersionProbe {
    fn name(&self) -> &str;
    /// Native version/checkpoint/snapshot id for the tree at `root`, if any.
    fn version(&self, root: &Path) -> Option<String>;
}

/// Default probe for filesystems with no native version id.
pub struct NoVersion;

impl VersionProbe for NoVersion {
    fn name(&self) -> &str {
        "none"
    }
    fn version(&self, _root: &Path) -> Option<String> {
        None
    }
}

/// Per-card raw result (the EXPORT LINEAGE view; not persisted anywhere).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CardResult {
    pub id: String,
    /// `complete` or `fail`.
    pub outcome: String,
    pub final_state: CardState,
    pub transitions: usize,
    pub epoch: u64,
    pub committed_bytes: u64,
    pub torn_tail_detected: bool,
    pub replay_ok: bool,
    pub lineage: Vec<LineageStep>,
}

/// Result of one workload run.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// The `ryanlab.bench.v1` record.
    pub record: Value,
    pub replay_digest: String,
    pub cards: Vec<CardResult>,
    /// Every card detected its torn tail and replayed to the same state.
    pub ok: bool,
}

#[derive(Default)]
struct Samples {
    rename_us: Vec<f64>,
    commit_us: Vec<f64>,
    fsync_us: Vec<f64>,
    recovery_us: Vec<f64>,
}

struct Live {
    id: String,
    dir: PathBuf,
    state: CardState,
    failed: bool,
    mem: Vec<Record>,
    torn_detected: bool,
}

/// splitmix64: deterministic per-card choices without a RNG dependency.
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn card_path(root: &Path, state: CardState, id: &str) -> PathBuf {
    root.join(state.as_str()).join(format!("{id}.bop"))
}

fn elapsed_us(t0: Instant) -> f64 {
    t0.elapsed().as_secs_f64() * 1e6
}

fn percentile(v: &[f64], p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let idx = ((p / 100.0) * (s.len() - 1) as f64).round() as usize;
    Some(s[idx])
}

fn sync_dir(p: &Path) {
    // Best effort: some platforms cannot open directories for fsync.
    if let Ok(d) = fs::File::open(p) {
        let _ = d.sync_all();
    }
}

fn artifact(id: &str, seed: u64, n: usize) -> Vec<u8> {
    let mut h = blake3::Hasher::new();
    h.update(b"bop.bench.artifact.v1\0");
    h.update(id.as_bytes());
    h.update(&seed.to_le_bytes());
    let mut out = vec![0u8; n];
    h.finalize_xof().fill(&mut out);
    out
}

fn move_card(root: &Path, live: &mut Live, to: CardState, s: &mut Samples) -> anyhow::Result<()> {
    let dst = card_path(root, to, &live.id);
    let t0 = Instant::now();
    fs::rename(&live.dir, &dst)?;
    s.rename_us.push(elapsed_us(t0));
    live.dir = dst;
    Ok(())
}

fn commit_step(
    live: &mut Live,
    to: CardState,
    content_hash: [u8; 32],
    s: &mut Samples,
) -> anyhow::Result<()> {
    let from = live.mem.last().map(|r| r.to);
    let p = Proposal {
        object_id: translog::object_id_for(&live.id),
        request_id: translog::request_id_for(&live.id, WORKLOAD),
        from,
        to,
        content_hash,
    };
    let t0 = Instant::now();
    match translog::commit(&live.dir, &p)? {
        Commit::Appended(r) => {
            s.commit_us.push(elapsed_us(t0));
            live.mem.push(r);
            live.state = to;
            Ok(())
        }
        Commit::AlreadyCommitted(r) => {
            anyhow::bail!("{}: unexpected idempotent commit at tid {}", live.id, r.tid)
        }
    }
}

/// Run the workload in `root`, which must be absent or empty.
pub fn run(root: &Path, cfg: &Config, probe: &dyn VersionProbe) -> anyhow::Result<Outcome> {
    anyhow::ensure!(cfg.cards > 0, "cards must be > 0");
    if root.exists() {
        anyhow::ensure!(
            fs::read_dir(root)?.next().is_none(),
            "bench dir {} must be empty or absent",
            root.display()
        );
    }
    for st in [
        CardState::Pending,
        CardState::Running,
        CardState::Done,
        CardState::Merged,
        CardState::Failed,
    ] {
        fs::create_dir_all(root.join(st.as_str()))?;
    }

    let version_before = probe.version(root);
    let mut s = Samples::default();
    let mut artifact_total: u64 = 0;
    let mut cards: Vec<Live> = Vec::with_capacity(cfg.cards);

    for i in 0..cfg.cards {
        let id = format!("bench-{i:05}");
        let failed = splitmix64(cfg.seed ^ (i as u64)).is_multiple_of(4);
        let mut live = Live {
            dir: card_path(root, CardState::Pending, &id),
            id,
            state: CardState::Pending,
            failed,
            mem: Vec::new(),
            torn_detected: false,
        };

        // CREATE
        fs::create_dir_all(live.dir.join("logs"))?;
        fs::create_dir_all(live.dir.join("output"))?;
        let meta = format!(
            "{{\"id\":\"{}\",\"stage\":\"bench\",\"seed\":{}}}\n",
            live.id, cfg.seed
        );
        fs::write(live.dir.join("meta.json"), &meta)?;
        let meta_hash = *blake3::hash(meta.as_bytes()).as_bytes();
        commit_step(&mut live, CardState::Pending, meta_hash, &mut s)?;

        // CLAIM: pending -> running
        move_card(root, &mut live, CardState::Running, &mut s)?;
        commit_step(&mut live, CardState::Running, meta_hash, &mut s)?;

        // APPEND logs
        let log_file = live.dir.join("logs").join("stdout.log");
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)?;
        for n in 0..cfg.log_lines {
            log.write_all(format!("bench {} line {n}\n", live.id).as_bytes())?;
        }
        drop(log);

        // WRITE artifact
        let art = artifact(&live.id, cfg.seed, cfg.artifact_bytes);
        let art_file = live.dir.join("output").join("result.bin");
        fs::write(&art_file, &art)?;
        artifact_total += art.len() as u64;
        let art_hash = *blake3::hash(&art).as_bytes();

        // COMPLETE | FAIL
        let end = if live.failed {
            CardState::Failed
        } else {
            CardState::Done
        };
        move_card(root, &mut live, end, &mut s)?;
        commit_step(&mut live, end, art_hash, &mut s)?;

        // FSYNC (paths moved with the rename)
        let t0 = Instant::now();
        fs::File::open(live.dir.join("output").join("result.bin"))?.sync_all()?;
        fs::File::open(live.dir.join("logs").join("stdout.log"))?.sync_all()?;
        sync_dir(&root.join(end.as_str()));
        s.fsync_us.push(elapsed_us(t0));

        // CRASH: a torn partial frame after the last committed transition.
        let frame = live.mem.last().expect("committed").frame();
        let cut =
            1 + (splitmix64(cfg.seed.rotate_left(17) ^ (i as u64)) as usize % (frame.len() - 1));
        OpenOptions::new()
            .append(true)
            .open(translog::log_path(&live.dir))?
            .write_all(&frame[..cut])?;

        // RECOVER: the torn tail is reported, never treated as committed.
        let t0 = Instant::now();
        let rec = translog::recover(&live.dir)?;
        s.recovery_us.push(elapsed_us(t0));
        live.torn_detected = rec.torn_tail.is_some() && rec.records == live.mem;

        // Resume: the next transition truncates the tail and bumps the epoch.
        let next = if live.failed {
            CardState::Pending
        } else {
            CardState::Merged
        };
        move_card(root, &mut live, next, &mut s)?;
        commit_step(&mut live, next, art_hash, &mut s)?;

        cards.push(live);
    }

    // QUERY HISTORY + EXPORT LINEAGE over the whole tree, from disk only.
    let mut digest = blake3::Hasher::new();
    digest.update(b"bop.bench.digest.v1\0");
    let mut results = Vec::with_capacity(cards.len());
    let mut log_bytes: u64 = 0;
    let t0 = Instant::now();
    for live in &cards {
        let data = fs::read(translog::log_path(&live.dir))?;
        let rec = translog::recover_bytes(&data);
        let expected = translog::project(&live.mem)?;
        let dir_state = live
            .dir
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .and_then(CardState::parse);
        let view = rec.view.clone();
        let replay_ok = rec.torn_tail.is_none()
            && rec.records == live.mem
            && view == expected
            && view.as_ref().map(|v| v.state) == Some(live.state)
            && dir_state == Some(live.state);

        digest.update(live.id.as_bytes());
        digest.update(b"\0");
        for r in &rec.records {
            digest.update(&r.encode());
        }
        digest.update(&[live.state.code()]);

        log_bytes += rec.committed_len;
        results.push(CardResult {
            id: live.id.clone(),
            outcome: if live.failed { "fail" } else { "complete" }.into(),
            final_state: live.state,
            transitions: rec.records.len(),
            epoch: view.as_ref().map(|v| v.epoch).unwrap_or(0),
            committed_bytes: rec.committed_len,
            torn_tail_detected: live.torn_detected,
            replay_ok,
            lineage: view.map(|v| v.lineage).unwrap_or_default(),
        });
    }
    let lineage_ms = t0.elapsed().as_secs_f64() * 1e3;
    let version_after = probe.version(root);

    let replay_digest = digest.finalize().to_hex().to_string();
    let torn = results.iter().filter(|c| c.torn_tail_detected).count();
    let mismatches = results.iter().filter(|c| !c.replay_ok).count();
    let transitions: usize = results.iter().map(|c| c.transitions).sum();
    let ok = torn == results.len() && mismatches == 0;

    let mut tags = Map::new();
    let mut tag = |k: &str, v: String| {
        tags.insert(k.into(), Value::String(v));
    };
    tag("seed", cfg.seed.to_string());
    tag("cards", cfg.cards.to_string());
    tag("artifact_bytes", cfg.artifact_bytes.to_string());
    tag("log_lines", cfg.log_lines.to_string());
    tag("version_probe", probe.name().to_string());
    tag("replay_digest", replay_digest.clone());
    if let Some(v) = &version_before {
        tag("fs_version_before", v.clone());
    }
    if let Some(v) = &version_after {
        tag("fs_version_after", v.clone());
    }

    let record = json!({
        "schema": BENCH_SCHEMA,
        "project": cfg.project,
        "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "runtime": cfg.runtime,
        "filesystem": cfg.filesystem,
        "workload": WORKLOAD,
        "commit": cfg.commit,
        "artifact_sha256": null,
        "metrics": {
            "artifact_bytes": artifact_total,
            "rss_bytes": null,
            "boot_ms": null,
            "rename_us_p50": percentile(&s.rename_us, 50.0),
            "fsync_us_p50": percentile(&s.fsync_us, 50.0),
            "recovery_ms": s.recovery_us.iter().sum::<f64>() / 1e3,
            "write_amplification": null,
            "metadata_bytes_per_run": log_bytes as f64 / cfg.cards as f64,
            "history_bytes_per_gib": null,
            "lineage_reconstruction_ms": lineage_ms,
            "cards": cfg.cards,
            "transitions": transitions,
            "commit_us_p50": percentile(&s.commit_us, 50.0),
            "commit_us_p99": percentile(&s.commit_us, 99.0),
            "torn_tails_detected": torn,
            "replay_mismatches": mismatches,
        },
        "tags": Value::Object(tags),
        "notes": format!("steps: {}", STEPS.join(" > ")),
        "raw": {
            "ok": ok,
            "steps": STEPS,
            "cards": &results,
            "samples_us": {
                "rename": &s.rename_us,
                "commit": &s.commit_us,
                "fsync": &s.fsync_us,
                "recovery": &s.recovery_us,
            },
        },
    });

    Ok(Outcome {
        record,
        replay_digest,
        cards: results,
        ok,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn cfg(cards: usize, seed: u64) -> Config {
        Config {
            cards,
            seed,
            artifact_bytes: 256,
            log_lines: 4,
            ..Config::default()
        }
    }

    #[test]
    fn same_config_same_digest_in_different_roots() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let x = run(&a.path().join("t"), &cfg(12, 42), &NoVersion).unwrap();
        let y = run(&b.path().join("t"), &cfg(12, 42), &NoVersion).unwrap();
        assert!(x.ok && y.ok);
        assert_eq!(x.replay_digest, y.replay_digest);
        assert_eq!(x.cards, y.cards);
    }

    #[test]
    fn seed_changes_digest() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let x = run(a.path(), &cfg(8, 1), &NoVersion).unwrap();
        let y = run(b.path(), &cfg(8, 2), &NoVersion).unwrap();
        assert_ne!(x.replay_digest, y.replay_digest);
    }

    #[test]
    fn every_card_crashes_recovers_and_resumes() {
        let d = tempdir().unwrap();
        let out = run(d.path(), &cfg(32, 7), &NoVersion).unwrap();
        assert!(out.ok);
        assert_eq!(out.cards.len(), 32);
        for c in &out.cards {
            assert!(c.torn_tail_detected, "{}", c.id);
            assert!(c.replay_ok, "{}", c.id);
            // Crash recovery bumped the epoch exactly once.
            assert_eq!(c.epoch, 2, "{}", c.id);
            assert_eq!(c.transitions, 4, "{}", c.id);
            assert_eq!(c.committed_bytes, 4 * translog::FRAME_LEN as u64);
            let ops: Vec<translog::Op> = c.lineage.iter().map(|s| s.op).collect();
            use translog::Op::*;
            if c.outcome == "fail" {
                assert_eq!(ops, [Create, Claim, Fail, Retry]);
                assert_eq!(c.final_state, CardState::Pending);
            } else {
                assert_eq!(ops, [Create, Claim, Complete, Merge]);
                assert_eq!(c.final_state, CardState::Merged);
            }
            assert!(card_path(d.path(), c.final_state, &c.id).is_dir());
        }
        assert_eq!(out.record["metrics"]["torn_tails_detected"], 32);
        assert_eq!(out.record["metrics"]["replay_mismatches"], 0);
    }

    #[test]
    fn record_has_bench_v1_shape() {
        let d = tempdir().unwrap();
        let mut c = cfg(4, 3);
        c.filesystem = Some("tmpfs".into());
        c.commit = Some("abc123".into());
        let r = run(d.path(), &c, &NoVersion).unwrap().record;
        for k in ["schema", "project", "timestamp", "workload", "metrics"] {
            assert!(r.get(k).is_some(), "missing {k}");
        }
        assert_eq!(r["schema"], BENCH_SCHEMA);
        assert_eq!(r["project"], "bop");
        assert_eq!(r["workload"], WORKLOAD);
        assert_eq!(r["filesystem"], "tmpfs");
        assert_eq!(r["commit"], "abc123");
        assert!(r["runtime"].is_null());
        assert!(r["artifact_sha256"].is_null());
        assert!(chrono::DateTime::parse_from_rfc3339(r["timestamp"].as_str().unwrap()).is_ok());
        for (k, v) in r["metrics"].as_object().unwrap() {
            assert!(
                v.is_null() || v.as_f64().map(|x| x >= 0.0).unwrap_or(false),
                "metric {k} = {v}"
            );
        }
        for (k, v) in r["tags"].as_object().unwrap() {
            assert!(v.is_string(), "tag {k} not a string");
        }
        assert_eq!(r["metrics"]["artifact_bytes"], 4 * 256);
    }

    #[test]
    fn refuses_non_empty_dir() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("keep"), b"x").unwrap();
        assert!(run(d.path(), &cfg(1, 1), &NoVersion).is_err());
        assert!(d.path().join("keep").exists());
    }

    struct Fixed;
    impl VersionProbe for Fixed {
        fn name(&self) -> &str {
            "fixed"
        }
        fn version(&self, _root: &Path) -> Option<String> {
            Some("tid-1".into())
        }
    }

    #[test]
    fn probe_versions_are_tagged_and_do_not_change_digest() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let x = run(a.path(), &cfg(3, 5), &Fixed).unwrap();
        let y = run(b.path(), &cfg(3, 5), &NoVersion).unwrap();
        assert_eq!(x.record["tags"]["version_probe"], "fixed");
        assert_eq!(x.record["tags"]["fs_version_before"], "tid-1");
        assert_eq!(x.record["tags"]["fs_version_after"], "tid-1");
        assert!(y.record["tags"].get("fs_version_before").is_none());
        assert_eq!(x.replay_digest, y.replay_digest);
    }
}
