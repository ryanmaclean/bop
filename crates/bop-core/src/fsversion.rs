//! Filesystem-version / run identity interface (ryanmaclean/bop#7).
//!
//! Binds a BOP run to the version/history identity of the filesystem it ran on,
//! without BOP becoming a filesystem implementation or a lineage database.
//!
//! Ownership split:
//!
//! - **BOP owns** the run id, the parent run id, the card and its committed
//!   transitions (the `translog` records: tid, epoch, op, hash chain).
//! - **The filesystem owns** object/path identity, transaction/checkpoint/
//!   snapshot identity, and crash consistency. It reports that identity
//!   through [`FsVersionBackend`] and never alters BOP records.
//!
//! The directory state machine stays authoritative. A [`RunBinder`] only
//! observes transitions that were already committed and renamed, and records
//! which filesystem version each one landed in. Paths are projections: nothing
//! here takes a path as identity. There is no Git/JJ dependency.
//!
//! Backends choose their own granularity. HAMMER1 can report a native TID per
//! transition. A snapshot filesystem reports versions only at run boundaries,
//! so this interface never forces a snapshot per transition. LFS reports the
//! checkpoint serial a transition landed in. [`shapes`] has one deterministic
//! reference fixture per shape, which real adapters are tested against.

use serde::{Deserialize, Serialize};

use crate::translog::{Op, Record};

/// JSON schema id for a serialized [`RunBinding`].
pub const RUN_SCHEMA: &str = "bop.fsversion.run.v1";

/// BOP-owned identity of one execution attempt of one card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunIdentity {
    pub run_id: String,
    /// The previous attempt of the same request (retry lineage), if any.
    pub parent_run_id: Option<String>,
    pub card_id: String,
    /// Hex `request_id` from the transition log; stable across retries.
    pub request_id: String,
    /// 1-based execution attempt (the translog `Claim` count).
    pub attempt: u32,
}

fn uuid_like(b: &[u8; 32]) -> String {
    let h: String = b[..16].iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Deterministic run id: blake3 of `(card, request, attempt)`, UUID-shaped.
/// No clocks and no randomness, so a replayed log yields the same ids.
pub fn run_id_for(card_id: &str, request_id_hex: &str, attempt: u32) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"bop.run.v1\0");
    h.update(card_id.as_bytes());
    h.update(b"\0");
    h.update(request_id_hex.as_bytes());
    h.update(&attempt.to_le_bytes());
    uuid_like(h.finalize().as_bytes())
}

impl RunIdentity {
    /// Identity of `attempt` (>= 1); the parent is attempt - 1.
    pub fn for_attempt(card_id: &str, request_id_hex: &str, attempt: u32) -> Self {
        let attempt = attempt.max(1);
        RunIdentity {
            run_id: run_id_for(card_id, request_id_hex, attempt),
            parent_run_id: (attempt > 1).then(|| run_id_for(card_id, request_id_hex, attempt - 1)),
            card_id: card_id.to_string(),
            request_id: request_id_hex.to_string(),
            attempt,
        }
    }
}

/// What kind of native identity a backend reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionKind {
    /// Transaction id (HAMMER1 TID, HAMMER2 modify_tid).
    Tid,
    /// Named snapshot (HAMMER2 PFS snapshot, FFS snapshot).
    Snapshot,
    /// Checkpoint serial (NetBSD LFS).
    Checkpoint,
}

/// A filesystem-native version identity, opaque to BOP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsVersion {
    pub backend: String,
    pub kind: VersionKind,
    pub id: String,
}

/// The backend contract. Each call may return `None` when the filesystem has
/// no native identity at that boundary.
pub trait FsVersionBackend {
    fn name(&self) -> &str;
    fn begin_run(&mut self, run: &RunIdentity) -> anyhow::Result<Option<FsVersion>>;
    /// Called after `rec` is durably committed (and the card renamed).
    fn commit_transition(
        &mut self,
        run: &RunIdentity,
        rec: &Record,
    ) -> anyhow::Result<Option<FsVersion>>;
    fn end_run(&mut self, run: &RunIdentity) -> anyhow::Result<Option<FsVersion>>;
}

/// Backend for filesystems without native version identity.
pub struct NoBackend;

impl FsVersionBackend for NoBackend {
    fn name(&self) -> &str {
        "none"
    }
    fn begin_run(&mut self, _run: &RunIdentity) -> anyhow::Result<Option<FsVersion>> {
        Ok(None)
    }
    fn commit_transition(
        &mut self,
        _run: &RunIdentity,
        _rec: &Record,
    ) -> anyhow::Result<Option<FsVersion>> {
        Ok(None)
    }
    fn end_run(&mut self, _run: &RunIdentity) -> anyhow::Result<Option<FsVersion>> {
        Ok(None)
    }
}

/// One committed transition and the filesystem version it landed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionBinding {
    pub tid: u64,
    pub epoch: u64,
    pub op: Op,
    /// blake3 of the canonical record body (links back to the log).
    pub record_hash: String,
    pub fs_version: Option<FsVersion>,
}

/// Facts binding one run to filesystem history. An export view (e.g. for an
/// OpenLineage projection), not a database: it is rebuildable from the
/// transition log plus the backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBinding {
    pub schema: String,
    pub run: RunIdentity,
    pub backend: String,
    pub before: Option<FsVersion>,
    pub transitions: Vec<TransitionBinding>,
    pub after: Option<FsVersion>,
}

/// Drives `begin_run` / `commit_transition` / `end_run` for one run.
pub struct RunBinder<'a> {
    backend: &'a mut dyn FsVersionBackend,
    binding: RunBinding,
}

impl<'a> RunBinder<'a> {
    pub fn begin(backend: &'a mut dyn FsVersionBackend, run: RunIdentity) -> anyhow::Result<Self> {
        let before = backend.begin_run(&run)?;
        let name = backend.name().to_string();
        Ok(RunBinder {
            backend,
            binding: RunBinding {
                schema: RUN_SCHEMA.into(),
                run,
                backend: name,
                before,
                transitions: Vec::new(),
                after: None,
            },
        })
    }

    /// Observe a committed record. Records must arrive in log order.
    pub fn committed(&mut self, rec: &Record) -> anyhow::Result<()> {
        if let Some(last) = self.binding.transitions.last() {
            anyhow::ensure!(
                rec.tid > last.tid,
                "record tid {} is not after {}",
                rec.tid,
                last.tid
            );
        }
        let fs_version = self.backend.commit_transition(&self.binding.run, rec)?;
        self.binding.transitions.push(TransitionBinding {
            tid: rec.tid,
            epoch: rec.epoch,
            op: rec.op,
            record_hash: blake3::Hash::from(rec.hash()).to_hex().to_string(),
            fs_version,
        });
        Ok(())
    }

    pub fn end(mut self) -> anyhow::Result<RunBinding> {
        self.binding.after = self.backend.end_run(&self.binding.run)?;
        Ok(self.binding)
    }
}

/// Deterministic reference fixtures, one per backend shape. They model the
/// identity semantics only (no I/O), so real adapters can be compared against
/// them on BSD hosts.
pub mod shapes {
    use super::*;

    /// HAMMER1-like: every committed transition gets a new, strictly
    /// increasing native TID. Run boundaries report the current TID.
    pub struct TidShape {
        pub tid: u64,
    }

    impl TidShape {
        pub fn new(start: u64) -> Self {
            TidShape { tid: start }
        }
        fn current(&self) -> FsVersion {
            FsVersion {
                backend: "tid".into(),
                kind: VersionKind::Tid,
                id: format!("0x{:016x}", self.tid),
            }
        }
    }

    impl FsVersionBackend for TidShape {
        fn name(&self) -> &str {
            "tid"
        }
        fn begin_run(&mut self, _run: &RunIdentity) -> anyhow::Result<Option<FsVersion>> {
            Ok(Some(self.current()))
        }
        fn commit_transition(
            &mut self,
            _run: &RunIdentity,
            _rec: &Record,
        ) -> anyhow::Result<Option<FsVersion>> {
            self.tid += 1;
            Ok(Some(self.current()))
        }
        fn end_run(&mut self, _run: &RunIdentity) -> anyhow::Result<Option<FsVersion>> {
            Ok(Some(self.current()))
        }
    }

    /// HAMMER2 PFS / FFS-snapshot-like: snapshots only at run boundaries;
    /// individual transitions have no native identity.
    pub struct SnapshotShape;

    impl FsVersionBackend for SnapshotShape {
        fn name(&self) -> &str {
            "snapshot"
        }
        fn begin_run(&mut self, run: &RunIdentity) -> anyhow::Result<Option<FsVersion>> {
            Ok(Some(FsVersion {
                backend: "snapshot".into(),
                kind: VersionKind::Snapshot,
                id: format!("bop-{}-before", run.run_id),
            }))
        }
        fn commit_transition(
            &mut self,
            _run: &RunIdentity,
            _rec: &Record,
        ) -> anyhow::Result<Option<FsVersion>> {
            Ok(None)
        }
        fn end_run(&mut self, run: &RunIdentity) -> anyhow::Result<Option<FsVersion>> {
            Ok(Some(FsVersion {
                backend: "snapshot".into(),
                kind: VersionKind::Snapshot,
                id: format!("bop-{}-after", run.run_id),
            }))
        }
    }

    /// NetBSD LFS-like: a checkpoint serial that advances every `every`
    /// segment writes, so several transitions can share one checkpoint.
    pub struct CheckpointShape {
        pub serial: u64,
        pub writes: u64,
        pub every: u64,
    }

    impl CheckpointShape {
        pub fn new(every: u64) -> Self {
            CheckpointShape {
                serial: 1,
                writes: 0,
                every: every.max(1),
            }
        }
        fn current(&self) -> FsVersion {
            FsVersion {
                backend: "checkpoint".into(),
                kind: VersionKind::Checkpoint,
                id: self.serial.to_string(),
            }
        }
    }

    impl FsVersionBackend for CheckpointShape {
        fn name(&self) -> &str {
            "checkpoint"
        }
        fn begin_run(&mut self, _run: &RunIdentity) -> anyhow::Result<Option<FsVersion>> {
            Ok(Some(self.current()))
        }
        fn commit_transition(
            &mut self,
            _run: &RunIdentity,
            _rec: &Record,
        ) -> anyhow::Result<Option<FsVersion>> {
            self.writes += 1;
            let v = self.current();
            if self.writes.is_multiple_of(self.every) {
                self.serial += 1;
            }
            Ok(Some(v))
        }
        fn end_run(&mut self, _run: &RunIdentity) -> anyhow::Result<Option<FsVersion>> {
            // A run boundary forces a checkpoint.
            self.serial += 1;
            self.writes = 0;
            Ok(Some(self.current()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::shapes::*;
    use super::*;
    use crate::translog::{self, CardState, Commit, Proposal};
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    use CardState::*;
    const STEPS: &[(Option<CardState>, CardState)] = &[
        (None, Pending),
        (Some(Pending), Running),
        (Some(Running), Done),
        (Some(Done), Merged),
    ];

    /// Run the fixed lifecycle, renaming the card between state dirs like the
    /// dispatcher does, and bind it with `backend`.
    fn drive(root: &Path, backend: &mut dyn FsVersionBackend, rename: bool) -> RunBinding {
        let id = "card-fsv";
        let req = translog::request_id_for(id, "2026-09-24T00:00:00+00:00");
        let req_hex: String = req.iter().map(|b| format!("{b:02x}")).collect();
        let mut dir = root.join("pending").join("card-fsv.bop");
        fs::create_dir_all(&dir).unwrap();
        let mut binder =
            RunBinder::begin(backend, RunIdentity::for_attempt(id, &req_hex, 1)).unwrap();
        for (from, to) in STEPS {
            if rename && from.is_some() {
                let next = root.join(to.as_str()).join("card-fsv.bop");
                fs::create_dir_all(next.parent().unwrap()).unwrap();
                fs::rename(&dir, &next).unwrap();
                dir = next;
            }
            let p = Proposal {
                object_id: translog::object_id_for(id),
                request_id: req,
                from: *from,
                to: *to,
                content_hash: [3u8; 32],
            };
            let rec = match translog::commit(&dir, &p).unwrap() {
                Commit::Appended(r) => r,
                Commit::AlreadyCommitted(r) => r,
            };
            binder.committed(&rec).unwrap();
        }
        binder.end().unwrap()
    }

    fn twice<B: FsVersionBackend>(mk: impl Fn() -> B) -> (RunBinding, RunBinding) {
        let (a, b) = (tempdir().unwrap(), tempdir().unwrap());
        let x = drive(a.path(), &mut mk(), true);
        let y = drive(b.path(), &mut mk(), true);
        (x, y)
    }

    #[test]
    fn run_ids_are_deterministic_and_chain_retries() {
        let a1 = RunIdentity::for_attempt("c", "ab", 1);
        let a2 = RunIdentity::for_attempt("c", "ab", 2);
        assert_eq!(a1, RunIdentity::for_attempt("c", "ab", 1));
        assert_eq!(a1.parent_run_id, None);
        assert_eq!(a2.parent_run_id.as_deref(), Some(a1.run_id.as_str()));
        assert_ne!(a1.run_id, a2.run_id);
        assert_eq!(a1.run_id.len(), 36);
    }

    #[test]
    fn tid_shape_fixture() {
        let (x, y) = twice(|| TidShape::new(0x100));
        assert_eq!(x, y);
        assert_eq!(x.before.as_ref().unwrap().id, "0x0000000000000100");
        let ids: Vec<&str> = x
            .transitions
            .iter()
            .map(|t| t.fs_version.as_ref().unwrap().id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "0x0000000000000101",
                "0x0000000000000102",
                "0x0000000000000103",
                "0x0000000000000104"
            ]
        );
        assert_eq!(x.after.unwrap().id, "0x0000000000000104");
    }

    #[test]
    fn snapshot_shape_fixture_has_no_per_transition_snapshot() {
        let (x, y) = twice(|| SnapshotShape);
        assert_eq!(x, y);
        assert!(x.transitions.iter().all(|t| t.fs_version.is_none()));
        let before = x.before.unwrap();
        assert_eq!(before.kind, VersionKind::Snapshot);
        assert!(before.id.ends_with("-before"));
        assert!(x.after.unwrap().id.ends_with("-after"));
    }

    #[test]
    fn checkpoint_shape_fixture_shares_checkpoints() {
        let (x, y) = twice(|| CheckpointShape::new(2));
        assert_eq!(x, y);
        let ids: Vec<&str> = x
            .transitions
            .iter()
            .map(|t| t.fs_version.as_ref().unwrap().id.as_str())
            .collect();
        assert_eq!(ids, ["1", "1", "2", "2"]);
        assert_eq!(x.after.unwrap().id, "4");
    }

    #[test]
    fn backends_never_change_bop_facts_and_paths_are_not_identity() {
        let d = [tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap()];
        let runs = [
            drive(d[0].path(), &mut NoBackend, true),
            drive(d[1].path(), &mut TidShape::new(7), false),
            drive(d[2].path(), &mut CheckpointShape::new(3), true),
        ];
        let facts = |b: &RunBinding| {
            b.transitions
                .iter()
                .map(|t| (t.tid, t.epoch, t.op, t.record_hash.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(facts(&runs[0]), facts(&runs[1]));
        assert_eq!(facts(&runs[0]), facts(&runs[2]));
        assert_eq!(runs[0].run, runs[1].run);
        // The committed facts replay to the same state whatever the backend.
        let log = |p: &Path, st: &str| translog::recover(&p.join(st).join("card-fsv.bop")).unwrap();
        let a = log(d[0].path(), "merged");
        let b = log(d[1].path(), "pending");
        assert_eq!(a.records, b.records);
        assert_eq!(
            translog::project(&a.records).unwrap().unwrap().state,
            Merged
        );
    }

    #[test]
    fn binding_serializes_with_schema_and_rejects_out_of_order() {
        let d = tempdir().unwrap();
        let b = drive(d.path(), &mut TidShape::new(1), true);
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["schema"], RUN_SCHEMA);
        assert_eq!(v["transitions"][0]["op"], "create");
        assert_eq!(v["transitions"][0]["fs_version"]["kind"], "tid");
        let back: RunBinding = serde_json::from_value(v).unwrap();
        assert_eq!(back, b);

        let mut nb = NoBackend;
        let mut binder = RunBinder::begin(&mut nb, RunIdentity::for_attempt("c", "ab", 1)).unwrap();
        let rec = translog::recover(&d.path().join("merged").join("card-fsv.bop"))
            .unwrap()
            .records;
        binder.committed(&rec[1]).unwrap();
        assert!(binder.committed(&rec[0]).is_err());
    }
}
