//! Immutable per-card transition log + deterministic state projection.
//!
//! Scoped experiment for ryanmaclean/bop#9. The directory a card lives in
//! (`pending/`, `running/`, ...) stays authoritative; this module records the
//! same transitions as append-only committed facts inside the card bundle
//! (`logs/transitions.bin`) and derives the current state by replaying them.
//!
//! Design follows the shared constraints (ryanmaclean/skills docs/CONSTRAINTS.md)
//! and the durable-tid scaffold (scaffolds/durable-tid/docs/DESIGN.md):
//!
//! - **C2** identity (`object_id`, `request_id`), order (`tid`), content
//!   (`content_hash`) and presentation (directory path) are separate fields.
//! - **C5/C10** no wall-clock time participates in the record or the reducer.
//!   Recovery stops at the last frame whose length, checksum and hash-chain all
//!   verify; a torn tail is reported and never reinterpreted as committed.
//! - **C6** retries (`Requeue`, `Retry`) keep the original `request_id`;
//!   re-submitting a transition that is already the committed head is a no-op.
//! - **C12** the path is a projection; the log travels with the bundle on
//!   `rename`, so identity does not depend on where the card currently sits.
//!
//! Canonical record encoding (v1), little-endian, fixed layout, 154 bytes:
//!
//! ```text
//! off  size  field
//!   0     4  magic  "BOPT"
//!   4     2  version (u16) = 1
//!   6     1  op (u8)
//!   7     1  from state (u8, 0 = none)
//!   8     1  to state (u8)
//!   9     1  reserved = 0
//!  10     8  epoch (u64)
//!  18     8  tid (u64, first record = 1, strictly +1)
//!  26     8  parent_tid (u64, previous record's tid, 0 for the first)
//!  34     4  attempt (u32, number of Claim records so far)
//!  38     4  reserved = 0
//!  42    16  request_id
//!  58    32  object_id   (blake3 of "bop.card.v1\0" + card id)
//!  90    32  content_hash (blake3 of meta.json at commit, or zeros)
//! 122    32  prev_hash   (blake3 of previous record body, zeros for first)
//! ```
//!
//! On disk each record is framed as `u32 LE body_len | body | blake3(body)`
//! (190 bytes per transition for v1).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Current canonical encoding version.
pub const RECORD_VERSION: u16 = 1;
/// Size of a v1 record body in bytes.
pub const BODY_LEN: usize = 154;
/// Size of one on-disk frame (length prefix + body + blake3 checksum).
pub const FRAME_LEN: usize = 4 + BODY_LEN + 32;
/// Filename of the log inside `<card>/logs/`.
pub const LOG_FILE: &str = "transitions.bin";
/// Schema id for the JSON projection.
pub const VIEW_SCHEMA: &str = "bop.translog.view.v1";

const MAGIC: &[u8; 4] = b"BOPT";

// ── Vocabulary ───────────────────────────────────────────────────────────────

/// Card states, mirroring the state directories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardState {
    Drafts,
    Pending,
    Running,
    Done,
    Merged,
    Failed,
}

impl CardState {
    pub fn code(self) -> u8 {
        match self {
            CardState::Drafts => 1,
            CardState::Pending => 2,
            CardState::Running => 3,
            CardState::Done => 4,
            CardState::Merged => 5,
            CardState::Failed => 6,
        }
    }

    pub fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            1 => CardState::Drafts,
            2 => CardState::Pending,
            3 => CardState::Running,
            4 => CardState::Done,
            5 => CardState::Merged,
            6 => CardState::Failed,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CardState::Drafts => "drafts",
            CardState::Pending => "pending",
            CardState::Running => "running",
            CardState::Done => "done",
            CardState::Merged => "merged",
            CardState::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "drafts" => CardState::Drafts,
            "pending" => CardState::Pending,
            "running" => CardState::Running,
            "done" => CardState::Done,
            "merged" => CardState::Merged,
            "failed" => CardState::Failed,
            _ => return None,
        })
    }
}

/// Transition operations. The op is derived from `(from, to)` so callers
/// cannot record an op that disagrees with the state change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// First record: card enters `drafts` or `pending`.
    Create,
    /// drafts -> pending
    Promote,
    /// pending -> running (a new execution attempt)
    Claim,
    /// running -> pending (transport/provider retry; same request identity)
    Requeue,
    /// running -> done
    Complete,
    /// running|done -> failed
    Fail,
    /// failed|done -> pending (operator retry; same request identity)
    Retry,
    /// done -> merged
    Merge,
}

impl Op {
    pub fn code(self) -> u8 {
        match self {
            Op::Create => 1,
            Op::Promote => 2,
            Op::Claim => 3,
            Op::Requeue => 4,
            Op::Complete => 5,
            Op::Fail => 6,
            Op::Retry => 7,
            Op::Merge => 8,
        }
    }

    pub fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            1 => Op::Create,
            2 => Op::Promote,
            3 => Op::Claim,
            4 => Op::Requeue,
            5 => Op::Complete,
            6 => Op::Fail,
            7 => Op::Retry,
            8 => Op::Merge,
            _ => return None,
        })
    }

    /// The only legal op for a state change, or `None` if the change is illegal.
    pub fn for_transition(from: Option<CardState>, to: CardState) -> Option<Op> {
        use CardState::*;
        Some(match (from, to) {
            (None, Drafts) | (None, Pending) => Op::Create,
            (Some(Drafts), Pending) => Op::Promote,
            (Some(Pending), Running) => Op::Claim,
            (Some(Running), Pending) => Op::Requeue,
            (Some(Running), Done) => Op::Complete,
            (Some(Running), Failed) | (Some(Done), Failed) => Op::Fail,
            (Some(Failed), Pending) | (Some(Done), Pending) => Op::Retry,
            (Some(Done), Merged) => Op::Merge,
            _ => return None,
        })
    }
}

// ── Record + canonical encoding ──────────────────────────────────────────────

/// One committed transition fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub epoch: u64,
    pub tid: u64,
    pub parent_tid: u64,
    pub op: Op,
    pub from: Option<CardState>,
    pub to: CardState,
    pub attempt: u32,
    pub request_id: [u8; 16],
    pub object_id: [u8; 32],
    pub content_hash: [u8; 32],
    pub prev_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("record body has length {0}, expected {BODY_LEN}")]
    Length(usize),
    #[error("bad magic")]
    Magic,
    #[error("unsupported record version {0}")]
    Version(u16),
    #[error("non-zero reserved field")]
    Reserved,
    #[error("unknown op code {0}")]
    Op(u8),
    #[error("unknown state code {0}")]
    State(u8),
}

impl Record {
    /// Canonical v1 encoding (see module docs). Never uses native struct layout.
    pub fn encode(&self) -> [u8; BODY_LEN] {
        let mut b = [0u8; BODY_LEN];
        b[0..4].copy_from_slice(MAGIC);
        b[4..6].copy_from_slice(&RECORD_VERSION.to_le_bytes());
        b[6] = self.op.code();
        b[7] = self.from.map(CardState::code).unwrap_or(0);
        b[8] = self.to.code();
        b[9] = 0;
        b[10..18].copy_from_slice(&self.epoch.to_le_bytes());
        b[18..26].copy_from_slice(&self.tid.to_le_bytes());
        b[26..34].copy_from_slice(&self.parent_tid.to_le_bytes());
        b[34..38].copy_from_slice(&self.attempt.to_le_bytes());
        // 38..42 reserved (zero)
        b[42..58].copy_from_slice(&self.request_id);
        b[58..90].copy_from_slice(&self.object_id);
        b[90..122].copy_from_slice(&self.content_hash);
        b[122..154].copy_from_slice(&self.prev_hash);
        b
    }

    pub fn decode(b: &[u8]) -> Result<Record, DecodeError> {
        if b.len() != BODY_LEN {
            return Err(DecodeError::Length(b.len()));
        }
        if &b[0..4] != MAGIC {
            return Err(DecodeError::Magic);
        }
        let version = u16::from_le_bytes([b[4], b[5]]);
        if version != RECORD_VERSION {
            return Err(DecodeError::Version(version));
        }
        if b[9] != 0 || b[38..42] != [0u8; 4] {
            return Err(DecodeError::Reserved);
        }
        let op = Op::from_code(b[6]).ok_or(DecodeError::Op(b[6]))?;
        let from = match b[7] {
            0 => None,
            c => Some(CardState::from_code(c).ok_or(DecodeError::State(c))?),
        };
        let to = CardState::from_code(b[8]).ok_or(DecodeError::State(b[8]))?;
        let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        Ok(Record {
            epoch: u64_at(10),
            tid: u64_at(18),
            parent_tid: u64_at(26),
            op,
            from,
            to,
            attempt: u32::from_le_bytes(b[34..38].try_into().unwrap()),
            request_id: b[42..58].try_into().unwrap(),
            object_id: b[58..90].try_into().unwrap(),
            content_hash: b[90..122].try_into().unwrap(),
            prev_hash: b[122..154].try_into().unwrap(),
        })
    }

    /// Hash of the canonical body; the next record's `prev_hash`.
    pub fn hash(&self) -> [u8; 32] {
        *blake3::hash(&self.encode()).as_bytes()
    }

    /// Full on-disk frame: `u32 len | body | blake3(body)`.
    pub fn frame(&self) -> Vec<u8> {
        let body = self.encode();
        let mut out = Vec::with_capacity(FRAME_LEN);
        out.extend_from_slice(&(BODY_LEN as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(blake3::hash(&body).as_bytes());
        out
    }
}

// ── Identity helpers ─────────────────────────────────────────────────────────

/// Stable object identity for a card id.
pub fn object_id_for(card_id: &str) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"bop.card.v1\0");
    h.update(card_id.as_bytes());
    *h.finalize().as_bytes()
}

/// Stable request identity for a card: derived once from the card id and its
/// creation stamp. Retries never mint a new one (C6).
pub fn request_id_for(card_id: &str, created_rfc3339: &str) -> [u8; 16] {
    let mut h = blake3::Hasher::new();
    h.update(b"bop.request.v1\0");
    h.update(card_id.as_bytes());
    h.update(b"\0");
    h.update(created_rfc3339.as_bytes());
    h.finalize().as_bytes()[..16].try_into().unwrap()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ── Deterministic reducer ────────────────────────────────────────────────────

/// Current state reconstructed purely from committed records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardView {
    pub object_id: String,
    pub request_id: String,
    pub state: CardState,
    /// Number of `Claim` transitions (execution attempts).
    pub attempt: u32,
    pub epoch: u64,
    pub last_tid: u64,
    /// blake3 of the last record body: `(epoch, last_tid, head_hash)` is the
    /// trusted head in durable-tid terms.
    pub head_hash: String,
    pub lineage: Vec<LineageStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageStep {
    pub tid: u64,
    pub op: Op,
    pub from: Option<CardState>,
    pub to: CardState,
    pub attempt: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplayError {
    #[error("first record must be Create, got {0:?}")]
    FirstNotCreate(Op),
    #[error("tid {got} does not follow {prev}")]
    TidGap { prev: u64, got: u64 },
    #[error("tid {tid}: parent_tid {got} != previous tid {prev}")]
    Parent { tid: u64, prev: u64, got: u64 },
    #[error("tid {tid}: prev_hash does not match previous record")]
    Chain { tid: u64 },
    #[error("tid {tid}: epoch went backwards")]
    Epoch { tid: u64 },
    #[error("tid {tid}: request/object identity changed")]
    Identity { tid: u64 },
    #[error("tid {tid}: from-state {from:?} != current state {current:?}")]
    FromMismatch {
        tid: u64,
        from: Option<CardState>,
        current: Option<CardState>,
    },
    #[error("tid {tid}: illegal transition {from:?} -> {to:?} as {op:?}")]
    Illegal {
        tid: u64,
        op: Op,
        from: Option<CardState>,
        to: CardState,
    },
    #[error("tid {tid}: attempt {got} != expected {want}")]
    Attempt { tid: u64, want: u32, got: u32 },
}

/// Pure transition function: `apply(prev, record) -> next`. No clocks, no I/O.
pub fn apply(prev: Option<&CardView>, r: &Record) -> Result<CardView, ReplayError> {
    let current = prev.map(|v| v.state);
    match prev {
        None => {
            if r.op != Op::Create {
                return Err(ReplayError::FirstNotCreate(r.op));
            }
            if r.tid != 1 {
                return Err(ReplayError::TidGap {
                    prev: 0,
                    got: r.tid,
                });
            }
            if r.parent_tid != 0 {
                return Err(ReplayError::Parent {
                    tid: r.tid,
                    prev: 0,
                    got: r.parent_tid,
                });
            }
            if r.prev_hash != [0u8; 32] {
                return Err(ReplayError::Chain { tid: r.tid });
            }
        }
        Some(v) => {
            if r.tid != v.last_tid + 1 {
                return Err(ReplayError::TidGap {
                    prev: v.last_tid,
                    got: r.tid,
                });
            }
            if r.parent_tid != v.last_tid {
                return Err(ReplayError::Parent {
                    tid: r.tid,
                    prev: v.last_tid,
                    got: r.parent_tid,
                });
            }
            if hex(&r.prev_hash) != v.head_hash {
                return Err(ReplayError::Chain { tid: r.tid });
            }
            if r.epoch < v.epoch {
                return Err(ReplayError::Epoch { tid: r.tid });
            }
            if hex(&r.request_id) != v.request_id || hex(&r.object_id) != v.object_id {
                return Err(ReplayError::Identity { tid: r.tid });
            }
        }
    }
    if r.from != current {
        return Err(ReplayError::FromMismatch {
            tid: r.tid,
            from: r.from,
            current,
        });
    }
    if Op::for_transition(r.from, r.to) != Some(r.op) {
        return Err(ReplayError::Illegal {
            tid: r.tid,
            op: r.op,
            from: r.from,
            to: r.to,
        });
    }
    let prev_attempt = prev.map(|v| v.attempt).unwrap_or(0);
    let want = if r.op == Op::Claim {
        prev_attempt + 1
    } else {
        prev_attempt
    };
    if r.attempt != want {
        return Err(ReplayError::Attempt {
            tid: r.tid,
            want,
            got: r.attempt,
        });
    }
    let mut lineage = prev.map(|v| v.lineage.clone()).unwrap_or_default();
    lineage.push(LineageStep {
        tid: r.tid,
        op: r.op,
        from: r.from,
        to: r.to,
        attempt: r.attempt,
    });
    Ok(CardView {
        object_id: hex(&r.object_id),
        request_id: hex(&r.request_id),
        state: r.to,
        attempt: r.attempt,
        epoch: r.epoch,
        last_tid: r.tid,
        head_hash: hex(&r.hash()),
        lineage,
    })
}

/// Fold an ordered sequence of committed records into the current view.
pub fn project(records: &[Record]) -> Result<Option<CardView>, ReplayError> {
    let mut view: Option<CardView> = None;
    for r in records {
        view = Some(apply(view.as_ref(), r)?);
    }
    Ok(view)
}

// ── Log file: recovery + append ──────────────────────────────────────────────

/// Bytes after the last verifiable frame. Never treated as committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TornTail {
    pub offset: u64,
    pub bytes: u64,
    pub reason: String,
}

/// Result of scanning a log from the crash boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct Recovered {
    pub records: Vec<Record>,
    pub view: Option<CardView>,
    /// Length of the committed prefix in bytes.
    pub committed_len: u64,
    pub torn_tail: Option<TornTail>,
}

pub fn log_path(card_dir: &Path) -> PathBuf {
    card_dir.join("logs").join(LOG_FILE)
}

/// Scan raw log bytes. Stops at the first frame that is incomplete, fails its
/// checksum, fails to decode, or does not extend the committed chain.
pub fn recover_bytes(data: &[u8]) -> Recovered {
    let mut records = Vec::new();
    let mut view: Option<CardView> = None;
    let mut off = 0usize;
    let mut torn = None;
    while off < data.len() {
        let fail = |reason: String| TornTail {
            offset: off as u64,
            bytes: (data.len() - off) as u64,
            reason,
        };
        if data.len() - off < 4 {
            torn = Some(fail("incomplete length prefix".into()));
            break;
        }
        let len = u32::from_le_bytes(data[off..off + 4].try_into().unwrap()) as usize;
        if len != BODY_LEN {
            torn = Some(fail(format!("unexpected body length {len}")));
            break;
        }
        let end = off + 4 + len + 32;
        if end > data.len() {
            torn = Some(fail("incomplete frame".into()));
            break;
        }
        let body = &data[off + 4..off + 4 + len];
        let sum = &data[off + 4 + len..end];
        if blake3::hash(body).as_bytes() != sum {
            torn = Some(fail("checksum mismatch".into()));
            break;
        }
        let rec = match Record::decode(body) {
            Ok(r) => r,
            Err(e) => {
                torn = Some(fail(format!("decode: {e}")));
                break;
            }
        };
        match apply(view.as_ref(), &rec) {
            Ok(v) => view = Some(v),
            Err(e) => {
                torn = Some(fail(format!("replay: {e}")));
                break;
            }
        }
        records.push(rec);
        off = end;
    }
    Recovered {
        records,
        view,
        committed_len: off as u64,
        torn_tail: torn,
    }
}

/// Read and recover the log for a card bundle (missing file = empty log).
pub fn recover(card_dir: &Path) -> std::io::Result<Recovered> {
    match fs::read(log_path(card_dir)) {
        Ok(data) => Ok(recover_bytes(&data)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(recover_bytes(&[])),
        Err(e) => Err(e),
    }
}

/// Outcome of a commit request.
#[derive(Debug, Clone, PartialEq)]
pub enum Commit {
    /// A new record was durably appended.
    Appended(Record),
    /// The requested transition is already the committed head (idempotent retry).
    AlreadyCommitted(Record),
}

#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("transition rejected: {0}")]
    Rejected(#[from] ReplayError),
}

/// Identity and content supplied by the caller for a transition.
#[derive(Debug, Clone)]
pub struct Proposal {
    pub object_id: [u8; 32],
    pub request_id: [u8; 16],
    pub from: Option<CardState>,
    pub to: CardState,
    pub content_hash: [u8; 32],
}

/// Validate `p` against the recovered head and durably append it.
///
/// - If a torn tail exists it is truncated first and the epoch is bumped so
///   the crash boundary is visible in the history (recovery never invents a
///   committed transition, C10).
/// - If the head already records exactly this `from -> to` for the same
///   identity, nothing is written (C6).
/// - The record is written with a single `write_all` followed by `fsync`;
///   `Appended` is only returned after `sync_data` succeeds (C3/C7).
pub fn commit(card_dir: &Path, p: &Proposal) -> Result<Commit, CommitError> {
    let path = log_path(card_dir);
    let logs_dir = path.parent().expect("log path has parent");
    let created_dir = !logs_dir.exists();
    fs::create_dir_all(logs_dir)?;
    let rec = recover(card_dir)?;

    if let Some(last) = rec.records.last() {
        if last.from == p.from
            && last.to == p.to
            && last.request_id == p.request_id
            && last.object_id == p.object_id
        {
            return Ok(Commit::AlreadyCommitted(last.clone()));
        }
    }

    let mut epoch = rec.view.as_ref().map(|v| v.epoch).unwrap_or(1);
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)?;
    if rec.torn_tail.is_some() {
        file.set_len(rec.committed_len)?;
        file.sync_data()?;
        epoch += 1;
    }

    let prev = rec.view.as_ref();
    let op = Op::for_transition(p.from, p.to).ok_or(ReplayError::Illegal {
        tid: prev.map(|v| v.last_tid + 1).unwrap_or(1),
        op: Op::Create,
        from: p.from,
        to: p.to,
    })?;
    let attempt = prev.map(|v| v.attempt).unwrap_or(0) + u32::from(op == Op::Claim);
    let record = Record {
        epoch,
        tid: prev.map(|v| v.last_tid + 1).unwrap_or(1),
        parent_tid: prev.map(|v| v.last_tid).unwrap_or(0),
        op,
        from: p.from,
        to: p.to,
        attempt,
        request_id: p.request_id,
        object_id: p.object_id,
        content_hash: p.content_hash,
        prev_hash: rec.records.last().map(Record::hash).unwrap_or([0u8; 32]),
    };
    // Reject before writing: the reducer is the single definition of legality.
    apply(prev, &record)?;

    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(rec.committed_len))?;
    file.write_all(&record.frame())?;
    file.sync_data()?;
    if created_dir || rec.committed_len == 0 {
        // Make the new directory entry durable too (best-effort on platforms
        // where directories cannot be opened).
        if let Ok(d) = fs::File::open(logs_dir) {
            let _ = d.sync_all();
        }
    }
    Ok(Commit::Appended(record))
}

// ── Shadow mode wiring ───────────────────────────────────────────────────────

/// Env var that enables shadow writes from the dispatcher / merge gate / retry.
pub const ENV_ENABLE: &str = "BOP_TRANSLOG";

pub fn shadow_enabled() -> bool {
    matches!(
        std::env::var(ENV_ENABLE).ok().as_deref(),
        Some("1") | Some("true") | Some("on")
    )
}

/// Record a directory transition that has already happened (shadow mode).
///
/// If the log is empty, a `Create` into `from` is committed first so cards
/// created before the log existed still get a complete, replayable history.
/// Errors are returned for the caller to log; the directory state remains
/// authoritative either way.
pub fn shadow_transition(
    card_dir: &Path,
    meta: &crate::Meta,
    from: &str,
    to: &str,
) -> Result<Option<Commit>, CommitError> {
    let (Some(from_s), Some(to_s)) = (CardState::parse(from), CardState::parse(to)) else {
        return Ok(None);
    };
    let object_id = object_id_for(&meta.id);
    let request_id = request_id_for(&meta.id, &meta.created.to_rfc3339());
    let content_hash = fs::read(card_dir.join("meta.json"))
        .map(|b| *blake3::hash(&b).as_bytes())
        .unwrap_or([0u8; 32]);
    let rec = recover(card_dir)?;
    if rec.view.is_none() {
        let initial = if matches!(from_s, CardState::Drafts) {
            CardState::Drafts
        } else {
            CardState::Pending
        };
        commit(
            card_dir,
            &Proposal {
                object_id,
                request_id,
                from: None,
                to: initial,
                content_hash,
            },
        )?;
        // Bridge an unlogged prefix (e.g. a card first seen in done/) with
        // the canonical path so the explicit transition below applies.
        let mut cur = initial;
        for step in bridge_path(initial, from_s) {
            commit(
                card_dir,
                &Proposal {
                    object_id,
                    request_id,
                    from: Some(cur),
                    to: step,
                    content_hash,
                },
            )?;
            cur = step;
        }
    }
    commit(
        card_dir,
        &Proposal {
            object_id,
            request_id,
            from: Some(from_s),
            to: to_s,
            content_hash,
        },
    )
    .map(Some)
}

/// Shortest legal path from `a` to `b` along the canonical lifecycle, used
/// only to bootstrap cards that predate the log.
fn bridge_path(a: CardState, b: CardState) -> Vec<CardState> {
    use CardState::*;
    let chain = [Drafts, Pending, Running, Done, Merged];
    if b == Failed {
        let mut p = bridge_path(a, Running);
        p.push(Failed);
        return p;
    }
    let ia = chain.iter().position(|s| *s == a).unwrap_or(1);
    let ib = chain.iter().position(|s| *s == b).unwrap_or(1);
    if ib <= ia {
        return Vec::new();
    }
    chain[ia + 1..=ib].to_vec()
}

// ── Consistency check against the directory model ────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    pub schema: String,
    pub card: String,
    /// State implied by the directory the bundle lives in (authoritative today).
    pub dir_state: Option<CardState>,
    /// State reconstructed from the transition log.
    pub log_state: Option<CardState>,
    pub transitions: usize,
    pub log_bytes: u64,
    pub torn_tail: Option<TornTail>,
    pub consistent: bool,
}

pub const VERIFY_SCHEMA: &str = "bop.translog.verify.v1";

/// Compare the projection with the directory state of a card bundle.
pub fn verify(card_dir: &Path, card_id: &str) -> std::io::Result<VerifyReport> {
    let dir_state = card_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .and_then(CardState::parse);
    let rec = recover(card_dir)?;
    let log_state = rec.view.as_ref().map(|v| v.state);
    Ok(VerifyReport {
        schema: VERIFY_SCHEMA.into(),
        card: card_id.into(),
        dir_state,
        log_state,
        transitions: rec.records.len(),
        log_bytes: rec.committed_len,
        consistent: rec.torn_tail.is_none() && log_state.is_some() && log_state == dir_state,
        torn_tail: rec.torn_tail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn prop(from: Option<CardState>, to: CardState) -> Proposal {
        Proposal {
            object_id: object_id_for("card-a"),
            request_id: request_id_for("card-a", "2026-09-23T00:00:00+00:00"),
            from,
            to,
            content_hash: [7u8; 32],
        }
    }

    fn run(dir: &Path, steps: &[(Option<CardState>, CardState)]) {
        for (f, t) in steps {
            commit(dir, &prop(*f, *t)).unwrap();
        }
    }

    use CardState::*;
    const LIFECYCLE: &[(Option<CardState>, CardState)] = &[
        (None, Pending),
        (Some(Pending), Running),
        (Some(Running), Pending), // requeue (rate limit)
        (Some(Pending), Running),
        (Some(Running), Failed),
        (Some(Failed), Pending), // operator retry
        (Some(Pending), Running),
        (Some(Running), Done),
        (Some(Done), Merged),
    ];

    const GOLDEN_V1: &str = "d0f7e99d820340e79034646ecf1e045eeedfd67795fc629bf20d59b376a8fd7d";

    #[test]
    fn encoding_is_fixed_width_and_roundtrips() {
        let r = Record {
            epoch: 1,
            tid: 42,
            parent_tid: 41,
            op: Op::Claim,
            from: Some(Pending),
            to: Running,
            attempt: 3,
            request_id: [1; 16],
            object_id: [2; 32],
            content_hash: [3; 32],
            prev_hash: [4; 32],
        };
        let b = r.encode();
        assert_eq!(b.len(), BODY_LEN);
        assert_eq!(&b[0..4], b"BOPT");
        assert_eq!(&b[18..26], &42u64.to_le_bytes());
        assert_eq!(Record::decode(&b).unwrap(), r);
        assert_eq!(r.frame().len(), FRAME_LEN);
        assert_eq!(r.hash(), *blake3::hash(&b).as_bytes());
        // Golden hash pins the canonical v1 encoding: any layout change must
        // bump RECORD_VERSION and update this value deliberately.
        assert_eq!(hex(&r.hash()), GOLDEN_V1);
    }

    #[test]
    fn decode_rejects_bad_input() {
        let r = Record::decode(&[0u8; 10]);
        assert_eq!(r, Err(DecodeError::Length(10)));
        let mut b = [0u8; BODY_LEN];
        assert_eq!(Record::decode(&b), Err(DecodeError::Magic));
        b[0..4].copy_from_slice(b"BOPT");
        b[4] = 9;
        assert_eq!(Record::decode(&b), Err(DecodeError::Version(9)));
    }

    #[test]
    fn full_lifecycle_projects_state_attempts_and_lineage() {
        let d = tempdir().unwrap();
        run(d.path(), LIFECYCLE);
        let rec = recover(d.path()).unwrap();
        assert!(rec.torn_tail.is_none());
        let v = rec.view.unwrap();
        assert_eq!(v.state, Merged);
        assert_eq!(v.attempt, 3);
        assert_eq!(v.last_tid, LIFECYCLE.len() as u64);
        let ops: Vec<Op> = v.lineage.iter().map(|s| s.op).collect();
        assert_eq!(
            ops,
            vec![
                Op::Create,
                Op::Claim,
                Op::Requeue,
                Op::Claim,
                Op::Fail,
                Op::Retry,
                Op::Claim,
                Op::Complete,
                Op::Merge
            ]
        );
        assert_eq!(rec.committed_len, (FRAME_LEN * LIFECYCLE.len()) as u64);
    }

    #[test]
    fn retry_preserves_request_identity() {
        let d = tempdir().unwrap();
        run(d.path(), LIFECYCLE);
        let rec = recover(d.path()).unwrap();
        let first = rec.records[0].request_id;
        assert!(rec.records.iter().all(|r| r.request_id == first));
        // A proposal carrying a different request id is rejected.
        let mut p = prop(Some(Merged), Pending);
        p.request_id = [9; 16];
        assert!(commit(d.path(), &p).is_err());
    }

    #[test]
    fn replay_is_deterministic() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        run(a.path(), LIFECYCLE);
        run(b.path(), LIFECYCLE);
        let ba = fs::read(log_path(a.path())).unwrap();
        let bb = fs::read(log_path(b.path())).unwrap();
        assert_eq!(ba, bb, "same committed facts => byte-identical log");
        let va = recover_bytes(&ba).view;
        assert_eq!(va, recover_bytes(&bb).view);
        // Replaying twice from memory yields the same view.
        let recs = recover_bytes(&ba).records;
        assert_eq!(project(&recs).unwrap(), project(&recs).unwrap());
    }

    #[test]
    fn illegal_transitions_are_rejected_before_write() {
        let d = tempdir().unwrap();
        run(d.path(), &[(None, Pending)]);
        let before = fs::read(log_path(d.path())).unwrap();
        assert!(commit(d.path(), &prop(Some(Pending), Merged)).is_err());
        assert!(commit(d.path(), &prop(Some(Running), Done)).is_err()); // wrong from
        assert_eq!(fs::read(log_path(d.path())).unwrap(), before);
        // A log must start with Create.
        let e = tempdir().unwrap();
        assert!(commit(e.path(), &prop(Some(Pending), Running)).is_err());
    }

    #[test]
    fn resubmitting_committed_head_is_idempotent() {
        let d = tempdir().unwrap();
        run(d.path(), &[(None, Pending), (Some(Pending), Running)]);
        let len = fs::metadata(log_path(d.path())).unwrap().len();
        match commit(d.path(), &prop(Some(Pending), Running)).unwrap() {
            Commit::AlreadyCommitted(r) => assert_eq!(r.tid, 2),
            other => panic!("expected AlreadyCommitted, got {other:?}"),
        }
        assert_eq!(fs::metadata(log_path(d.path())).unwrap().len(), len);
    }

    #[test]
    fn torn_tail_is_ignored_then_truncated_with_epoch_bump() {
        let d = tempdir().unwrap();
        run(d.path(), &[(None, Pending), (Some(Pending), Running)]);
        let path = log_path(d.path());
        let full = fs::read(&path).unwrap();
        // Simulate a crash mid-write of a third frame (Complete).
        let third = Record {
            epoch: 1,
            tid: 3,
            parent_tid: 2,
            op: Op::Complete,
            from: Some(Running),
            to: Done,
            attempt: 1,
            request_id: prop(None, Pending).request_id,
            object_id: prop(None, Pending).object_id,
            content_hash: [0; 32],
            prev_hash: recover_bytes(&full).records[1].hash(),
        }
        .frame();
        for cut in [1, 4, 50, FRAME_LEN - 1] {
            let mut torn = full.clone();
            torn.extend_from_slice(&third[..cut]);
            let rec = recover_bytes(&torn);
            assert_eq!(rec.records.len(), 2, "cut={cut}");
            assert_eq!(
                rec.view.as_ref().unwrap().state,
                Running,
                "never invents Done"
            );
            assert_eq!(rec.torn_tail.as_ref().unwrap().bytes, cut as u64);
        }
        // Crash recovery then a new commit: tail truncated, epoch bumped.
        let mut torn = full.clone();
        torn.extend_from_slice(&third[..77]);
        fs::write(&path, &torn).unwrap();
        let out = commit(d.path(), &prop(Some(Running), Failed)).unwrap();
        let Commit::Appended(r) = out else {
            panic!("expected append")
        };
        assert_eq!((r.tid, r.epoch), (3, 2));
        let rec = recover(d.path()).unwrap();
        assert!(rec.torn_tail.is_none());
        assert_eq!(rec.view.unwrap().state, Failed);
    }

    #[test]
    fn corrupted_middle_frame_stops_replay_at_crash_boundary() {
        let d = tempdir().unwrap();
        run(d.path(), LIFECYCLE);
        let mut data = fs::read(log_path(d.path())).unwrap();
        data[FRAME_LEN * 3 + 20] ^= 0xff; // flip a byte inside record 4's body
        let rec = recover_bytes(&data);
        assert_eq!(rec.records.len(), 3);
        assert_eq!(rec.torn_tail.unwrap().reason, "checksum mismatch");
    }

    #[test]
    fn forged_frame_with_valid_checksum_but_broken_chain_is_rejected() {
        let d = tempdir().unwrap();
        run(d.path(), &[(None, Pending)]);
        let mut data = fs::read(log_path(d.path())).unwrap();
        let forged = Record {
            epoch: 1,
            tid: 2,
            parent_tid: 1,
            op: Op::Claim,
            from: Some(Pending),
            to: Running,
            attempt: 1,
            request_id: prop(None, Pending).request_id,
            object_id: prop(None, Pending).object_id,
            content_hash: [0; 32],
            prev_hash: [0xAA; 32],
        };
        data.extend_from_slice(&forged.frame());
        let rec = recover_bytes(&data);
        assert_eq!(rec.records.len(), 1);
        assert!(rec.torn_tail.unwrap().reason.starts_with("replay:"));
    }

    #[test]
    fn bridge_path_covers_legacy_cards() {
        assert_eq!(bridge_path(Pending, Pending), vec![]);
        assert_eq!(bridge_path(Pending, Done), vec![Running, Done]);
        assert_eq!(bridge_path(Pending, Failed), vec![Running, Failed]);
        assert_eq!(bridge_path(Drafts, Running), vec![Pending, Running]);
    }

    #[test]
    fn shadow_transition_bootstraps_and_verifies_against_directory() {
        let root = tempdir().unwrap();
        let done = root.path().join("done").join("legacy.bop");
        fs::create_dir_all(&done).unwrap();
        let meta = crate::Meta {
            id: "legacy".into(),
            ..Default::default()
        };
        // Card first observed moving done -> merged (no prior log).
        let merged = root.path().join("merged").join("legacy.bop");
        fs::create_dir_all(merged.parent().unwrap()).unwrap();
        fs::rename(&done, &merged).unwrap();
        shadow_transition(&merged, &meta, "done", "merged").unwrap();
        let rep = verify(&merged, "legacy").unwrap();
        assert!(rep.consistent, "{rep:?}");
        assert_eq!(rep.log_state, Some(Merged));
        assert_eq!(rep.transitions, 4); // create, claim, complete, merge
                                        // Unknown state names (e.g. "paused") are ignored, not errors.
        assert!(shadow_transition(&merged, &meta, "merged", "paused")
            .unwrap()
            .is_none());
    }

    #[test]
    fn verify_flags_divergence_between_directory_and_log() {
        let root = tempdir().unwrap();
        let card = root.path().join("running").join("x.bop");
        fs::create_dir_all(&card).unwrap();
        run(&card, &[(None, Pending)]); // log says pending, dir says running
        let rep = verify(&card, "x").unwrap();
        assert!(!rep.consistent);
        assert_eq!(rep.dir_state, Some(Running));
        assert_eq!(rep.log_state, Some(Pending));
    }
}
