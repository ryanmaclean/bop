# CLI Module Refactoring Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split `crates/jc/src/main.rs` (1531 lines) into per-command modules under `cmd/` and shared utilities under `util/`, remove dead stub crates, keeping all existing tests green.

**Architecture:** Keep `main.rs` as a thin dispatcher (~80 lines) containing only the `Cli`, `Command`, and `MemoryCommand` clap enums plus `main()`. All command logic moves to `crates/jc/src/cmd/<name>.rs`. Shared types and helper functions move to `crates/jc/src/util/<topic>.rs`.

**Tech Stack:** Rust, Cargo workspace, clap 4 (derive), tokio, anyhow. Tests are integration harness tests (`cargo test`) that invoke the compiled binary — they must pass unchanged. Linting enforced with `cargo clippy -- -D warnings`.

---

## Target Module Map

```
crates/jc/src/
  main.rs              (~80 lines)   — Cli, Command, MemoryCommand enums + main()
  cmd/
    mod.rs             (~5 lines)
    init.rs            (~55 lines)   — seed_default_templates(), pub fn run()
    new.rs             (~80 lines)   — pub fn run()
    status.rs          (~35 lines)   — pub fn run()
    validate.rs        (~20 lines)   — pub fn run()
    dispatcher.rs      (~255 lines)  — pub async fn run(), run_card()
    merge_gate.rs      (~185 lines)  — pub async fn run()
    retry.rs           (~40 lines)   — pub fn run()
    kill.rs            (~50 lines)   — pub async fn run()
    logs.rs            (~100 lines)  — pub async fn run(), print_log_section()
    inspect.rs         (~50 lines)   — pub fn run()
    memory.rs          (~65 lines)   — pub fn run(), cmd_memory_*()
  util/
    mod.rs             (~5 lines)
    fs.rs              (~110 lines)  — ensure_cards_layout, copy_dir_all, clone_template,
                                       find_card, find_card_in_state, append_log_line
    providers.rs       (~150 lines)  — Provider, ProvidersFile, all provider fns
    memory.rs          (~210 lines)  — MemoryStore types + all memory helper fns
    process.rs         (~95 lines)   — is_alive, read_pid, reap_orphans
```

**Key visibility rules:**
- Every function/type that crosses a module boundary must be `pub`.
- Use `crate::` for cross-module imports inside the `jc` crate.
- `MemoryCommand` stays in `main.rs` (it's a clap type referenced in `Command`); `cmd/memory.rs` imports it via `use crate::MemoryCommand`.

---

## Task 0: Establish Baseline

**Files:** none changed

**Step 1: Run the full test suite**

```bash
cd /Users/studio/gtfs
cargo test 2>&1 | tail -20
```

Expected: all tests pass (look for `test result: ok` lines, 0 failures).

**Step 2: Record line count for verification later**

```bash
wc -l crates/jc/src/main.rs
```

Expected: 1531 (or close).

---

## Task 1: Remove Dead Stub Crates

The crates `jc-dispatcher` and `jc-merge-gate` are NOT in `Cargo.toml`'s workspace members and contain only `anyhow::bail!("not implemented")`. Delete them.

**Files:**
- Delete: `crates/jc-dispatcher/` (entire directory)
- Delete: `crates/jc-merge-gate/` (entire directory)

**Step 1: Confirm they are not in the workspace**

```bash
grep -n "jc-dispatcher\|jc-merge-gate" /Users/studio/gtfs/Cargo.toml
```

Expected: no output (they are absent from workspace members).

**Step 2: Delete the stub crate directories**

```bash
rm -rf /Users/studio/gtfs/crates/jc-dispatcher
rm -rf /Users/studio/gtfs/crates/jc-merge-gate
```

**Step 3: Verify workspace still builds**

```bash
cargo check 2>&1 | tail -5
```

Expected: no errors.

**Step 4: Commit**

```bash
git -C /Users/studio/gtfs add -A crates/jc-dispatcher crates/jc-merge-gate
git -C /Users/studio/gtfs commit -m "chore: remove unimplemented jc-dispatcher and jc-merge-gate stubs"
```

---

## Task 2: Create util/ scaffolding

**Files:**
- Create: `crates/jc/src/util/mod.rs`

**Step 1: Create the util module file**

```rust
// crates/jc/src/util/mod.rs
pub mod fs;
pub mod memory;
pub mod process;
pub mod providers;
```

**Step 2: Create stub files so mod.rs compiles**

Create each with just an empty body for now:

```bash
touch /Users/studio/gtfs/crates/jc/src/util/fs.rs
touch /Users/studio/gtfs/crates/jc/src/util/memory.rs
touch /Users/studio/gtfs/crates/jc/src/util/process.rs
touch /Users/studio/gtfs/crates/jc/src/util/providers.rs
```

**Step 3: Wire into main.rs temporarily**

Add `mod util;` near the top of `crates/jc/src/main.rs` (after the existing use statements), then:

```bash
cargo check 2>&1 | tail -5
```

Expected: compiles (empty modules are valid).

---

## Task 3: Populate util/fs.rs

Move the filesystem helpers out of `main.rs`.

**Files:**
- Modify: `crates/jc/src/util/fs.rs`
- Modify: `crates/jc/src/main.rs` (replace bodies with `use crate::util::fs::*` or explicit imports, keep original fns temporarily as thin wrappers so rest of main.rs still compiles)

**Background:** The cleanest approach is to populate each util module fully FIRST, add `pub use` re-exports so the existing `main.rs` call-sites continue to resolve, THEN strip `main.rs` at the end (Task 18). This avoids mid-refactor compile failures.

**Step 1: Write util/fs.rs**

```rust
// crates/jc/src/util/fs.rs
use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use walkdir::WalkDir;

pub fn ensure_cards_layout(root: &Path) -> anyhow::Result<()> {
    for dir in ["templates", "pending", "running", "done", "merged", "failed", "memory"] {
        fs::create_dir_all(root.join(dir))?;
    }
    Ok(())
}

pub fn clone_template(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    if cfg!(target_os = "macos") {
        let status = StdCommand::new("cp").arg("-c").arg("-R").arg(src).arg(dst).status();
        if matches!(status, Ok(s) if s.success()) { return Ok(()); }
        let status = StdCommand::new("cp").arg("-R").arg(src).arg(dst).status();
        if matches!(status, Ok(s) if s.success()) { return Ok(()); }
    } else {
        let status = StdCommand::new("cp").arg("--reflink=auto").arg("-r").arg(src).arg(dst).status();
        if matches!(status, Ok(s) if s.success()) { return Ok(()); }
        let status = StdCommand::new("cp").arg("-r").arg(src).arg(dst).status();
        if matches!(status, Ok(s) if s.success()) { return Ok(()); }
    }
    copy_dir_all(src, dst)
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in WalkDir::new(src).min_depth(1) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

pub fn find_card(root: &Path, id: &str) -> Option<PathBuf> {
    let name = format!("{}.jobcard", id);
    for dir in ["pending", "running", "done", "merged", "failed"] {
        let p = root.join(dir).join(&name);
        if p.exists() { return Some(p); }
    }
    None
}

pub fn find_card_in_state(root: &Path, id: &str, state: &str) -> bool {
    root.join(state).join(format!("{}.jobcard", id)).exists()
}

pub fn append_log_line(path: &Path, line: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", line)?;
    Ok(())
}
```

**Step 2: In main.rs, forward-declare from util::fs**

At the top of `main.rs`, after `mod util;`, add:

```rust
use crate::util::fs::{
    append_log_line, clone_template, copy_dir_all, ensure_cards_layout,
    find_card, find_card_in_state,
};
```

Then **remove** the duplicate function bodies from `main.rs`:
- `fn ensure_cards_layout` (lines 172–185)
- `fn clone_template` (lines 187–231)
- `fn copy_dir_all` (lines 1294–1310)
- `fn find_card` (lines 1534–1543)
- `fn find_card_in_state` (lines 1484–1486)
- `fn append_log_line` (lines 603–607)

**Step 3: Verify compilation**

```bash
cargo check 2>&1 | grep -E "^error" | head -20
```

Expected: no errors. Fix any visibility or import issues before continuing.

**Step 4: Commit**

```bash
git -C /Users/studio/gtfs add crates/jc/src/util/ crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract filesystem utilities to util/fs.rs"
```

---

## Task 4: Populate util/providers.rs

Move all provider types and functions.

**Files:**
- Modify: `crates/jc/src/util/providers.rs`
- Modify: `crates/jc/src/main.rs`

**Step 1: Write util/providers.rs**

Copy the following from `main.rs` verbatim (adjust to add `pub`):

```rust
// crates/jc/src/util/providers.rs
use anyhow::Context;
use chrono::Utc;
use jobcard_core::Meta;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub command: String,
    #[serde(default)]
    pub rate_limit_exit: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until_epoch_s: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersFile {
    #[serde(default)]
    pub providers: BTreeMap<String, Provider>,
}

pub fn providers_path(cards_dir: &Path) -> PathBuf {
    cards_dir.join("providers.json")
}

pub fn read_providers(cards_dir: &Path) -> anyhow::Result<ProvidersFile> {
    let p = providers_path(cards_dir);
    if !p.exists() { return Ok(ProvidersFile::default()); }
    let bytes = fs::read(p)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn write_providers(cards_dir: &Path, pf: &ProvidersFile) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(pf)?;
    fs::write(providers_path(cards_dir), bytes)?;
    Ok(())
}

pub fn seed_providers(cards_dir: &Path) -> anyhow::Result<()> {
    let p = providers_path(cards_dir);
    if p.exists() { return Ok(()); }
    let mut pf = ProvidersFile::default();
    pf.providers.insert("mock".to_string(), Provider {
        command: "adapters/mock.sh".to_string(),
        rate_limit_exit: 75,
        cooldown_until_epoch_s: None,
    });
    pf.providers.insert("mock2".to_string(), Provider {
        command: "adapters/mock.sh".to_string(),
        rate_limit_exit: 75,
        cooldown_until_epoch_s: None,
    });
    write_providers(cards_dir, &pf)?;
    Ok(())
}

pub fn ensure_mock_provider_command(cards_dir: &Path, adapter: &str) -> anyhow::Result<()> {
    let mut pf = read_providers(cards_dir)?;
    if let Some(p) = pf.providers.get_mut("mock") {
        p.command = adapter.to_string();
    }
    write_providers(cards_dir, &pf)?;
    Ok(())
}

pub fn rotate_provider_chain(meta: &mut Meta) {
    if meta.provider_chain.len() <= 1 { return; }
    let first = meta.provider_chain.remove(0);
    meta.provider_chain.push(first);
}

pub fn select_provider(
    cards_dir: &Path,
    meta: Option<&mut Meta>,
    stage: &str,
) -> anyhow::Result<Option<(String, String, i32)>> {
    let pf = read_providers(cards_dir)?;
    let now = Utc::now().timestamp();
    let avoid_provider = if stage == "qa" {
        meta.as_ref()
            .and_then(|m| m.stages.get("implement"))
            .and_then(|r| r.provider.clone())
    } else {
        None
    };
    let chain: Vec<String> = match meta {
        Some(m) => {
            if m.provider_chain.is_empty() {
                m.provider_chain = vec!["mock".to_string(), "mock2".to_string()];
            }
            m.provider_chain.clone()
        }
        None => vec!["mock".to_string(), "mock2".to_string()],
    };
    let mut fallback: Option<(String, String)> = None;
    for name in chain {
        let Some(p) = pf.providers.get(&name) else { continue; };
        if let Some(until) = p.cooldown_until_epoch_s {
            if until > now { continue; }
        }
        if let Some(ref avoid) = avoid_provider {
            if &name == avoid {
                if fallback.is_none() { fallback = Some((name, p.command.clone())); }
                continue;
            }
        }
        return Ok(Some((name, p.command.clone(), p.rate_limit_exit)));
    }
    if let Some((name, cmd)) = fallback {
        if let Some(p) = pf.providers.get(&name) {
            return Ok(Some((name, cmd, p.rate_limit_exit)));
        }
    }
    Ok(None)
}

pub fn set_provider_cooldown(cards_dir: &Path, provider: &str, cooldown_s: i64) -> anyhow::Result<()> {
    let mut pf = read_providers(cards_dir)?;
    let now = Utc::now().timestamp();
    if let Some(p) = pf.providers.get_mut(provider) {
        p.cooldown_until_epoch_s = Some(now + cooldown_s);
    }
    write_providers(cards_dir, &pf)?;
    Ok(())
}
```

**Step 2: Update main.rs imports**

Add to the `use crate::util::*` section:

```rust
use crate::util::providers::{
    ensure_mock_provider_command, read_providers, rotate_provider_chain,
    seed_providers, select_provider, set_provider_cooldown, write_providers,
    Provider, ProvidersFile,
};
```

Remove from `main.rs` the following function bodies and type definitions:
- `struct Provider` + `struct ProvidersFile` (lines 242–255)
- `fn providers_path` (257–259)
- `fn read_providers` (261–268)
- `fn write_providers` (270–274)
- `fn seed_providers` (276–301)
- `fn ensure_mock_provider_command` (233–240)
- `fn rotate_provider_chain` (1038–1044)
- `fn select_provider` (1046–1101)
- `fn set_provider_cooldown` (1103–1111)

**Step 3: Verify compilation**

```bash
cargo check 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

**Step 4: Commit**

```bash
git -C /Users/studio/gtfs add crates/jc/src/util/providers.rs crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract provider types and logic to util/providers.rs"
```

---

## Task 5: Populate util/memory.rs

Move all memory-related types and helpers.

**Files:**
- Modify: `crates/jc/src/util/memory.rs`
- Modify: `crates/jc/src/main.rs`

**Step 1: Write util/memory.rs**

Copy verbatim from `main.rs`, adding `pub` to all types and functions:

```rust
// crates/jc/src/util/memory.rs
use anyhow::Context;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use jobcard_core::Meta;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_MEMORY_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryStore {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entries: BTreeMap<String, MemoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub value: String,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MemoryOutput {
    Ops(MemoryOutputOps),
    Flat(BTreeMap<String, MemoryOutputValue>),
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MemoryOutputOps {
    #[serde(default)]
    pub set: BTreeMap<String, MemoryOutputValue>,
    #[serde(default)]
    pub delete: Vec<String>,
    #[serde(default)]
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MemoryOutputValue {
    String(String),
    Detailed {
        value: String,
        #[serde(default)]
        ttl_seconds: Option<i64>,
    },
}

pub fn normalize_namespace(namespace: &str) -> String {
    let trimmed = namespace.trim();
    if trimmed.is_empty() { "default".to_string() } else { trimmed.to_string() }
}

pub fn sanitize_namespace(namespace: &str) -> String {
    let normalized = normalize_namespace(namespace);
    let sanitized: String = normalized.chars().map(|ch| {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' { ch } else { '_' }
    }).collect();
    if sanitized.is_empty() { "default".to_string() } else { sanitized }
}

pub fn memory_store_path(cards_dir: &Path, namespace: &str) -> PathBuf {
    cards_dir.join("memory").join(format!("{}.json", sanitize_namespace(namespace)))
}

pub fn prune_memory_store(store: &mut MemoryStore, now: DateTime<Utc>) -> usize {
    let before = store.entries.len();
    store.entries.retain(|_, entry| entry.expires_at.map(|exp| exp > now).unwrap_or(true));
    before.saturating_sub(store.entries.len())
}

pub fn read_memory_store(cards_dir: &Path, namespace: &str) -> anyhow::Result<MemoryStore> {
    let namespace = normalize_namespace(namespace);
    let path = memory_store_path(cards_dir, &namespace);
    if !path.exists() { return Ok(MemoryStore::default()); }
    let bytes = fs::read(&path)?;
    let mut store = if bytes.is_empty() {
        MemoryStore::default()
    } else {
        serde_json::from_slice::<MemoryStore>(&bytes)
            .with_context(|| format!("invalid memory store {}", path.display()))?
    };
    let pruned = prune_memory_store(&mut store, Utc::now());
    if pruned > 0 { write_memory_store(cards_dir, &namespace, &store)?; }
    Ok(store)
}

pub fn write_memory_store(cards_dir: &Path, namespace: &str, store: &MemoryStore) -> anyhow::Result<()> {
    fs::create_dir_all(cards_dir.join("memory"))?;
    let path = memory_store_path(cards_dir, namespace);
    let bytes = serde_json::to_vec_pretty(store)?;
    fs::write(path, bytes)?;
    Ok(())
}

pub fn set_memory_entry(store: &mut MemoryStore, key: &str, value: &str, ttl_seconds: i64, now: DateTime<Utc>) {
    let expires_at = now + ChronoDuration::seconds(ttl_seconds);
    store.entries.insert(key.to_string(), MemoryEntry {
        value: value.to_string(),
        updated_at: now,
        expires_at: Some(expires_at),
    });
}

pub fn format_memory_for_prompt(store: &MemoryStore) -> String {
    if store.entries.is_empty() { return String::new(); }
    let facts: BTreeMap<String, String> = store.entries.iter()
        .map(|(k, v)| (k.clone(), v.value.clone())).collect();
    serde_json::to_string_pretty(&facts).unwrap_or_default()
}

pub fn memory_namespace_from_meta(meta: &Meta) -> String {
    meta.template_namespace.as_deref()
        .map(normalize_namespace)
        .filter(|ns| !ns.is_empty())
        .unwrap_or_else(|| normalize_namespace(&meta.stage))
}

pub fn parse_memory_output(path: &Path) -> anyhow::Result<MemoryOutputOps> {
    let bytes = fs::read(path)?;
    if bytes.iter().all(|b| b.is_ascii_whitespace()) { return Ok(MemoryOutputOps::default()); }
    let parsed: MemoryOutput = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid memory output {}", path.display()))?;
    Ok(match parsed {
        MemoryOutput::Ops(ops) => ops,
        MemoryOutput::Flat(set) => MemoryOutputOps { set, delete: vec![], ttl_seconds: None },
    })
}

pub fn merge_memory_output(cards_dir: &Path, namespace: &str, path: &Path) -> anyhow::Result<()> {
    if !path.exists() { return Ok(()); }
    let ops = parse_memory_output(path)?;
    if ops.set.is_empty() && ops.delete.is_empty() { return Ok(()); }
    let mut store = read_memory_store(cards_dir, namespace)?;
    let now = Utc::now();
    for key in ops.delete {
        let key = key.trim();
        if !key.is_empty() { store.entries.remove(key); }
    }
    for (key, value) in ops.set {
        let key = key.trim();
        if key.is_empty() { continue; }
        let (value, item_ttl) = match value {
            MemoryOutputValue::String(v) => (v, None),
            MemoryOutputValue::Detailed { value, ttl_seconds } => (value, ttl_seconds),
        };
        let ttl_seconds = item_ttl.or(ops.ttl_seconds)
            .filter(|ttl| *ttl > 0).unwrap_or(DEFAULT_MEMORY_TTL_SECONDS);
        set_memory_entry(&mut store, key, &value, ttl_seconds, now);
    }
    let _ = prune_memory_store(&mut store, now);
    write_memory_store(cards_dir, namespace, &store)?;
    Ok(())
}
```

**Step 2: Update main.rs**

Add import:
```rust
use crate::util::memory::{
    DEFAULT_MEMORY_TTL_SECONDS, MemoryEntry, MemoryOutput, MemoryOutputOps,
    MemoryOutputValue, MemoryStore, format_memory_for_prompt, memory_namespace_from_meta,
    merge_memory_output, normalize_namespace, parse_memory_output, prune_memory_store,
    read_memory_store, sanitize_namespace, set_memory_entry, write_memory_store,
    memory_store_path,
};
```

Remove from `main.rs`:
- `const DEFAULT_MEMORY_TTL_SECONDS` (line 25)
- `struct MemoryStore`, `MemoryEntry`, `MemoryOutput`, `MemoryOutputOps`, `MemoryOutputValue` (lines 130–170)
- All memory helper function bodies (lines 431–601)

**Step 3: Verify**

```bash
cargo check 2>&1 | grep -E "^error" | head -20
```

**Step 4: Commit**

```bash
git -C /Users/studio/gtfs add crates/jc/src/util/memory.rs crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract memory types and helpers to util/memory.rs"
```

---

## Task 6: Populate util/process.rs

Move the OS-level process management helpers.

**Files:**
- Modify: `crates/jc/src/util/process.rs`
- Modify: `crates/jc/src/main.rs`

**Step 1: Write util/process.rs**

```rust
// crates/jc/src/util/process.rs
use jobcard_core::{write_meta, Meta};
use std::fs;
use std::path::Path;
use tokio::process::Command as TokioCommand;

pub async fn read_pid(card_dir: &Path) -> anyhow::Result<Option<i32>> {
    let out = TokioCommand::new("xattr")
        .arg("-p").arg("com.yourorg.agent-pid").arg(card_dir)
        .output().await;
    if let Ok(out) = out {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                if let Ok(pid) = s.trim().parse::<i32>() {
                    return Ok(Some(pid));
                }
            }
        }
    }
    let pid_path = card_dir.join("logs").join("pid");
    if let Ok(s) = fs::read_to_string(pid_path) {
        if let Ok(pid) = s.trim().parse::<i32>() {
            return Ok(Some(pid));
        }
    }
    Ok(None)
}

pub async fn is_alive(pid: i32) -> anyhow::Result<bool> {
    let status = TokioCommand::new("kill").arg("-0").arg(pid.to_string()).status().await?;
    Ok(status.success())
}

pub async fn reap_orphans(
    running_dir: &Path,
    pending_dir: &Path,
    failed_dir: &Path,
    max_retries: u32,
) -> anyhow::Result<()> {
    let entries = match fs::read_dir(running_dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for ent in entries.flatten() {
        let card_dir = ent.path();
        if !card_dir.is_dir() { continue; }
        if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" { continue; }
        let pid = read_pid(&card_dir).await?;
        let Some(pid) = pid else { continue; };
        if is_alive(pid).await? { continue; }
        let mut meta = jobcard_core::read_meta(&card_dir).ok();
        let retry_count = meta.as_ref().and_then(|m| m.retry_count).unwrap_or(0);
        let next_retry = retry_count.saturating_add(1);
        if let Some(ref mut m) = meta {
            m.retry_count = Some(next_retry);
            if next_retry > max_retries {
                m.failure_reason = Some("max_retries_exceeded".to_string());
            }
            let _ = write_meta(&card_dir, m);
        }
        let name = match card_dir.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let target = if next_retry > max_retries {
            failed_dir.join(&name)
        } else {
            pending_dir.join(&name)
        };
        let _ = fs::rename(&card_dir, &target);
    }
    Ok(())
}
```

**Step 2: Update main.rs**

Add import:
```rust
use crate::util::process::{is_alive, read_pid, reap_orphans};
```

Remove from `main.rs`:
- `async fn reap_orphans` (lines 303–356)
- `async fn read_pid` (lines 358–383)
- `async fn is_alive` (lines 385–392)

**Step 3: Verify**

```bash
cargo check 2>&1 | grep -E "^error" | head -20
```

**Step 4: Commit**

```bash
git -C /Users/studio/gtfs add crates/jc/src/util/process.rs crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract process helpers (read_pid, is_alive, reap_orphans) to util/process.rs"
```

---

## Task 7: Create cmd/ scaffolding

**Files:**
- Create: `crates/jc/src/cmd/mod.rs`
- Create stub files for all commands

**Step 1: Create cmd/mod.rs**

```rust
// crates/jc/src/cmd/mod.rs
pub mod dispatcher;
pub mod init;
pub mod inspect;
pub mod kill;
pub mod logs;
pub mod memory;
pub mod merge_gate;
pub mod new;
pub mod retry;
pub mod status;
pub mod validate;
```

**Step 2: Create empty stub files**

```bash
for f in dispatcher init inspect kill logs memory merge_gate new retry status validate; do
  touch /Users/studio/gtfs/crates/jc/src/cmd/${f}.rs
done
```

**Step 3: Add mod cmd to main.rs**

Add `mod cmd;` near `mod util;` in `main.rs`.

**Step 4: Verify**

```bash
cargo check 2>&1 | grep -E "^error" | head -10
```

---

## Task 8: Create cmd/init.rs

**Files:**
- Modify: `crates/jc/src/cmd/init.rs`
- Modify: `crates/jc/src/main.rs`

**Step 1: Write cmd/init.rs**

Copy `seed_default_templates` (lines 394–429) and wrap `cmd_init` logic:

```rust
// crates/jc/src/cmd/init.rs
use anyhow::Context as _;
use chrono::Utc;
use jobcard_core::{write_meta, Meta};
use std::fs;
use std::path::Path;
use crate::util::fs::ensure_cards_layout;
use crate::util::providers::seed_providers;

pub fn run(root: &Path) -> anyhow::Result<()> {
    ensure_cards_layout(root)?;
    seed_default_templates(root)?;
    seed_providers(root)?;
    Ok(())
}

fn seed_default_templates(cards_dir: &Path) -> anyhow::Result<()> {
    let templates_dir = cards_dir.join("templates");
    let implement = templates_dir.join("implement.jobcard");
    if !implement.exists() {
        fs::create_dir_all(implement.join("logs"))?;
        fs::create_dir_all(implement.join("output"))?;
        let meta = Meta {
            id: "template-implement".to_string(),
            created: Utc::now(),
            agent_type: None,
            stage: "implement".to_string(),
            priority: None,
            provider_chain: vec![],
            stages: Default::default(),
            acceptance_criteria: vec![],
            worktree_branch: Some("job/template-implement".to_string()),
            template_namespace: Some("implement".to_string()),
            retry_count: Some(0),
            failure_reason: None,
            validation_summary: None,
        };
        write_meta(&implement, &meta)?;
        if !implement.join("spec.md").exists() { fs::write(implement.join("spec.md"), "")?; }
        if !implement.join("prompt.md").exists() {
            fs::write(implement.join("prompt.md"), "{{spec}}\n\nAcceptance criteria:\n{{acceptance_criteria}}\n")?;
        }
    }
    Ok(())
}
```

**Step 2: Update main.rs**

In the `Command::Init` match arm, replace the inline body with:

```rust
Command::Init => crate::cmd::init::run(&root),
```

Remove `fn seed_default_templates` from `main.rs` (lines 394–429). Remove now-unused imports.

**Step 3: Verify**

```bash
cargo check 2>&1 | grep -E "^error" | head -20
```

**Step 4: Commit**

```bash
git -C /Users/studio/gtfs add crates/jc/src/cmd/init.rs crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract init command to cmd/init.rs"
```

---

## Task 9: Create cmd/new.rs

**Files:**
- Modify: `crates/jc/src/cmd/new.rs`
- Modify: `crates/jc/src/main.rs`

**Step 1: Write cmd/new.rs**

```rust
// crates/jc/src/cmd/new.rs
use anyhow::Context as _;
use chrono::Utc;
use jobcard_core::{write_meta, Meta};
use std::fs;
use std::path::Path;
use crate::util::fs::{clone_template, ensure_cards_layout};

pub fn run(root: &Path, template: &str, id: &str) -> anyhow::Result<()> {
    ensure_cards_layout(root)?;
    let template_dir = root.join("templates").join(format!("{}.jobcard", template));
    if !template_dir.exists() {
        anyhow::bail!("template not found: {}", template);
    }
    let card_dir = root.join("pending").join(format!("{}.jobcard", id));
    if card_dir.exists() {
        anyhow::bail!("card already exists: {}", id);
    }
    clone_template(&template_dir, &card_dir)
        .with_context(|| format!("failed to clone template {}", template))?;
    fs::create_dir_all(card_dir.join("logs"))?;
    fs::create_dir_all(card_dir.join("output"))?;
    let mut meta = jobcard_core::read_meta(&card_dir).unwrap_or_else(|_| Meta {
        id: id.to_string(),
        created: Utc::now(),
        agent_type: None,
        stage: "spec".to_string(),
        priority: None,
        provider_chain: vec![],
        stages: Default::default(),
        acceptance_criteria: vec![],
        worktree_branch: Some(format!("job/{}", id)),
        template_namespace: Some(template.to_string()),
        retry_count: Some(0),
        failure_reason: None,
        validation_summary: None,
    });
    meta.id = id.to_string();
    meta.created = Utc::now();
    meta.worktree_branch = Some(format!("job/{}", id));
    meta.template_namespace = Some(template.to_string());
    meta.retry_count = Some(0);
    meta.failure_reason = None;
    write_meta(&card_dir, &meta)?;
    if !card_dir.join("spec.md").exists() { fs::write(card_dir.join("spec.md"), "")?; }
    if !card_dir.join("prompt.md").exists() { fs::write(card_dir.join("prompt.md"), "{{spec}}\n")?; }
    Ok(())
}
```

**Step 2: Update main.rs**

Replace `Command::New { template, id }` match arm body with:

```rust
Command::New { template, id } => crate::cmd::new::run(&root, &template, &id),
```

**Step 3: Verify + Commit**

```bash
cargo check 2>&1 | grep -E "^error" | head -10
git -C /Users/studio/gtfs add crates/jc/src/cmd/new.rs crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract new command to cmd/new.rs"
```

---

## Task 10: Create cmd/status.rs and cmd/validate.rs

**Files:**
- Modify: `crates/jc/src/cmd/status.rs`
- Modify: `crates/jc/src/cmd/validate.rs`
- Modify: `crates/jc/src/main.rs`

**Step 1: Write cmd/status.rs**

```rust
// crates/jc/src/cmd/status.rs
use anyhow::Context as _;
use std::fs;
use std::path::Path;
use crate::util::fs::find_card;

pub fn run(root: &Path, id: &str) -> anyhow::Result<()> {
    if id.trim().is_empty() {
        for dir in ["pending", "running", "done", "merged", "failed"] {
            let p = root.join(dir);
            if p.exists() {
                let count = fs::read_dir(&p).map(|rd| rd.count()).unwrap_or(0);
                println!("{}\t{}", dir, count);
            }
        }
        return Ok(());
    }
    let card = find_card(root, id).context("card not found")?;
    let meta = jobcard_core::read_meta(&card)?;
    println!("{}", serde_json::to_string_pretty(&meta)?);
    Ok(())
}
```

**Step 2: Write cmd/validate.rs**

```rust
// crates/jc/src/cmd/validate.rs
use anyhow::Context as _;
use std::path::Path;
use crate::util::fs::find_card;

pub fn run(root: &Path, id: &str, _realtime: bool) -> anyhow::Result<()> {
    let card = find_card(root, id).context("card not found")?;
    let _ = jobcard_core::read_meta(&card)?;
    Ok(())
}
```

**Step 3: Update main.rs match arms**

```rust
Command::Status { id } => crate::cmd::status::run(&root, &id),
Command::Validate { id, realtime } => crate::cmd::validate::run(&root, &id, realtime),
```

**Step 4: Verify + Commit**

```bash
cargo check 2>&1 | grep -E "^error" | head -10
git -C /Users/studio/gtfs add crates/jc/src/cmd/status.rs crates/jc/src/cmd/validate.rs crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract status and validate commands to cmd/"
```

---

## Task 11: Create cmd/dispatcher.rs

This is the largest module (~255 lines). It contains `run_dispatcher` and `run_card`.

**Files:**
- Modify: `crates/jc/src/cmd/dispatcher.rs`
- Modify: `crates/jc/src/main.rs`

**Step 1: Write cmd/dispatcher.rs**

Copy `run_dispatcher` (lines 797–917) and `run_card` (lines 919–1036) verbatim, adjusting imports:

```rust
// crates/jc/src/cmd/dispatcher.rs
use anyhow::Context as _;
use chrono::Utc;
use jobcard_core::{write_meta, PromptContext};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command as TokioCommand;
use crate::util::fs::ensure_cards_layout;
use crate::util::process::reap_orphans;
use crate::util::providers::{
    ensure_mock_provider_command, rotate_provider_chain, seed_providers,
    select_provider, set_provider_cooldown,
};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    cards_dir: &Path,
    adapter: &str,
    max_workers: usize,
    poll_ms: u64,
    max_retries: u32,
    reap_ms: u64,
    no_reap: bool,
    once: bool,
) -> anyhow::Result<()> {
    ensure_cards_layout(cards_dir)?;
    seed_providers(cards_dir)?;
    ensure_mock_provider_command(cards_dir, adapter)?;

    let pending_dir = cards_dir.join("pending");
    let running_dir = cards_dir.join("running");
    let done_dir = cards_dir.join("done");
    let failed_dir = cards_dir.join("failed");

    let mut last_reap = std::time::Instant::now()
        .checked_sub(Duration::from_millis(reap_ms))
        .unwrap_or_else(std::time::Instant::now);

    loop {
        if !no_reap && last_reap.elapsed() >= Duration::from_millis(reap_ms) {
            reap_orphans(&running_dir, &pending_dir, &failed_dir, max_retries).await?;
            last_reap = std::time::Instant::now();
        }

        let running_count = fs::read_dir(&running_dir).map(|rd| rd.count()).unwrap_or(0);
        let mut available_slots = max_workers.saturating_sub(running_count);

        if available_slots > 0 {
            if let Ok(entries) = fs::read_dir(&pending_dir) {
                for ent in entries.flatten() {
                    if available_slots == 0 { break; }
                    let pending_path = ent.path();
                    if !pending_path.is_dir() { continue; }
                    if pending_path.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" {
                        continue;
                    }
                    let name = match pending_path.file_name().and_then(|s| s.to_str()) {
                        Some(n) => n.to_string(),
                        None => continue,
                    };
                    let running_path = running_dir.join(&name);
                    if fs::rename(&pending_path, &running_path).is_err() { continue; }
                    available_slots = available_slots.saturating_sub(1);

                    let mut meta = jobcard_core::read_meta(&running_path).ok();
                    let stage = meta.as_ref()
                        .map(|m| m.stage.clone())
                        .unwrap_or_else(|| "implement".to_string());

                    let (provider_name, provider_cmd, rate_limit_exit) =
                        match select_provider(cards_dir, meta.as_mut(), &stage)? {
                            Some(v) => v,
                            None => {
                                let _ = fs::rename(&running_path, pending_dir.join(&name));
                                continue;
                            }
                        };

                    if let Some(ref mut m) = meta { let _ = write_meta(&running_path, m); }

                    let (exit_code, mut meta) =
                        run_card(&running_path, &provider_cmd, &provider_name)
                            .await.unwrap_or((1, None));

                    let is_rate_limited = exit_code == rate_limit_exit;
                    if let Some(ref mut meta) = meta {
                        if is_rate_limited {
                            let next = meta.retry_count.unwrap_or(0).saturating_add(1);
                            meta.retry_count = Some(next);
                            rotate_provider_chain(meta);
                            let _ = set_provider_cooldown(cards_dir, &provider_name, 300);
                        }
                        let _ = write_meta(&running_path, meta);
                    }

                    let target = if exit_code == 0 {
                        done_dir.join(&name)
                    } else if is_rate_limited {
                        pending_dir.join(&name)
                    } else {
                        failed_dir.join(&name)
                    };
                    let _ = fs::rename(&running_path, &target);
                }
            }
        }

        if once { break; }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }
    Ok(())
}

async fn run_card(
    card_dir: &Path,
    adapter: &str,
    provider_name: &str,
) -> anyhow::Result<(i32, Option<jobcard_core::Meta>)> {
    fs::create_dir_all(card_dir.join("logs"))?;
    fs::create_dir_all(card_dir.join("output"))?;

    let prompt_file = card_dir.join("prompt.md");
    if !prompt_file.exists() { fs::write(&prompt_file, "")?; }

    let mut meta = jobcard_core::read_meta(card_dir).ok();
    if let Some(ref m) = meta {
        let ctx = PromptContext::from_files(card_dir, m)?;
        let template = fs::read_to_string(&prompt_file)?;
        let rendered = jobcard_core::render_prompt(&template, &ctx);
        fs::write(&prompt_file, rendered)?;
    }

    let stdout_log = card_dir.join("logs").join("stdout.log");
    let stderr_log = card_dir.join("logs").join("stderr.log");
    let workdir = {
        let wt = card_dir.join("worktree");
        if wt.exists() { wt } else { card_dir.to_path_buf() }
    };

    let stage = meta.as_ref().map(|m| m.stage.clone()).unwrap_or_else(|| "implement".to_string());
    let started_at = Utc::now();
    if let Some(ref mut m) = meta {
        let rec = m.stages.entry(stage.clone()).or_insert(jobcard_core::StageRecord {
            status: jobcard_core::StageStatus::Pending,
            agent: None, provider: None, duration_s: None, started: None, blocked_by: None,
        });
        rec.status = jobcard_core::StageStatus::Running;
        rec.started = Some(started_at);
        rec.agent = Some(adapter.to_string());
        rec.provider = Some(provider_name.to_string());
        let _ = write_meta(card_dir, m);
    }

    let mut cmd = if adapter.ends_with(".sh") {
        let mut c = TokioCommand::new("bash");
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

    let mut child = cmd
        .arg(&workdir).arg(&prompt_file).arg(&stdout_log).arg(&stderr_log)
        .spawn()
        .with_context(|| format!("failed to spawn adapter: {}", adapter))?;

    if let Some(pid) = child.id() {
        let pid_str = pid.to_string();
        let _ = fs::write(card_dir.join("logs").join("pid"), &pid_str);
        let _ = TokioCommand::new("xattr")
            .arg("-w").arg("com.yourorg.agent-pid").arg(&pid_str).arg(card_dir)
            .status().await;
    }

    let status = child.wait().await?;
    let exit_code = status.code().unwrap_or(1);
    let finished_at = Utc::now();
    if let Some(ref mut m) = meta {
        let rec = m.stages.entry(stage).or_insert(jobcard_core::StageRecord {
            status: jobcard_core::StageStatus::Pending,
            agent: None, provider: None, duration_s: None, started: None, blocked_by: None,
        });
        rec.status = if exit_code == 0 {
            jobcard_core::StageStatus::Done
        } else if exit_code == 75 {
            jobcard_core::StageStatus::Pending
        } else {
            jobcard_core::StageStatus::Failed
        };
        let duration = finished_at.signed_duration_since(started_at).num_seconds();
        if duration >= 0 { rec.duration_s = Some(duration as u64); }
    }
    Ok((exit_code, meta))
}
```

**Step 2: Update main.rs**

Replace the `Command::Dispatcher { ... }` match arm with:

```rust
Command::Dispatcher { adapter, max_workers, poll_ms, max_retries, reap_ms, no_reap, once, .. } => {
    crate::cmd::dispatcher::run(&root, &adapter, max_workers, poll_ms, max_retries, reap_ms, no_reap, once).await
}
```

Remove `async fn run_dispatcher` and `async fn run_card` from `main.rs`.

**Step 3: Verify**

```bash
cargo check 2>&1 | grep -E "^error" | head -20
```

**Step 4: Commit**

```bash
git -C /Users/studio/gtfs add crates/jc/src/cmd/dispatcher.rs crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract dispatcher command to cmd/dispatcher.rs"
```

---

## Task 12: Create cmd/merge_gate.rs

**Files:**
- Modify: `crates/jc/src/cmd/merge_gate.rs`
- Modify: `crates/jc/src/main.rs`

**Step 1: Write cmd/merge_gate.rs**

Copy `run_merge_gate` (lines 1113–1292) verbatim, adjusting imports:

```rust
// crates/jc/src/cmd/merge_gate.rs
use jobcard_core::write_meta;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command as TokioCommand;
use crate::util::fs::ensure_cards_layout;

pub async fn run(cards_dir: &Path, poll_ms: u64, once: bool) -> anyhow::Result<()> {
    ensure_cards_layout(cards_dir)?;
    let done_dir = cards_dir.join("done");
    let merged_dir = cards_dir.join("merged");
    let failed_dir = cards_dir.join("failed");
    loop {
        // ... (full body of run_merge_gate, copied verbatim) ...
        if once { break; }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }
    Ok(())
}
```

> **Note for implementer:** Copy the full loop body from `main.rs` lines 1120–1291 verbatim. The structure is: iterate `done/`, run acceptance criteria as shell commands, run `git merge --no-ff`, move to `merged/` or `failed/`.

**Step 2: Update main.rs**

```rust
Command::MergeGate { poll_ms, once } => crate::cmd::merge_gate::run(&root, poll_ms, once).await,
```

Remove `async fn run_merge_gate` from `main.rs`.

**Step 3: Verify + Commit**

```bash
cargo check 2>&1 | grep -E "^error" | head -10
git -C /Users/studio/gtfs add crates/jc/src/cmd/merge_gate.rs crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract merge-gate command to cmd/merge_gate.rs"
```

---

## Task 13: Create cmd/retry.rs and cmd/kill.rs

**Files:**
- Modify: `crates/jc/src/cmd/retry.rs`
- Modify: `crates/jc/src/cmd/kill.rs`
- Modify: `crates/jc/src/main.rs`

**Step 1: Write cmd/retry.rs**

Copy `cmd_retry` (lines 1314–1341):

```rust
// crates/jc/src/cmd/retry.rs
use anyhow::Context as _;
use jobcard_core::write_meta;
use std::fs;
use std::path::Path;
use crate::util::fs::find_card;

pub fn run(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let state = card.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap_or("unknown");
    if state == "running" { anyhow::bail!("card '{}' is currently running; kill it first", id); }
    if state == "pending" { anyhow::bail!("card '{}' is already pending", id); }
    if let Ok(mut meta) = jobcard_core::read_meta(&card) {
        meta.retry_count = Some(meta.retry_count.unwrap_or(0).saturating_add(1));
        meta.failure_reason = None;
        let _ = write_meta(&card, &meta);
    }
    let target = root.join("pending").join(format!("{}.jobcard", id));
    fs::rename(&card, &target).with_context(|| format!("failed to move card to pending/: {}", id))?;
    println!("retrying: {} -> pending/", id);
    Ok(())
}
```

**Step 2: Write cmd/kill.rs**

Copy `cmd_kill` (lines 1345–1386):

```rust
// crates/jc/src/cmd/kill.rs
use anyhow::Context as _;
use jobcard_core::write_meta;
use std::fs;
use std::path::Path;
use tokio::process::Command as TokioCommand;
use crate::util::fs::find_card;
use crate::util::process::read_pid;

pub async fn run(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let state = card.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap_or("unknown");
    if state != "running" { anyhow::bail!("card '{}' is not running (state: {})", id, state); }
    let pid = read_pid(&card).await?.with_context(|| format!("no PID found for card '{}'", id))?;
    let sent = TokioCommand::new("kill").arg("-TERM").arg(pid.to_string())
        .status().await.with_context(|| format!("failed to send SIGTERM to pid {}", pid))?;
    if !sent.success() { anyhow::bail!("kill -TERM {} returned non-zero", pid); }
    if let Ok(mut meta) = jobcard_core::read_meta(&card) {
        meta.failure_reason = Some("killed".to_string());
        let _ = write_meta(&card, &meta);
    }
    let target = root.join("failed").join(format!("{}.jobcard", id));
    fs::rename(&card, &target).with_context(|| format!("failed to move card to failed/: {}", id))?;
    println!("killed pid {} and moved '{}' to failed/", pid, id);
    Ok(())
}
```

**Step 3: Update main.rs**

```rust
Command::Retry { id } => crate::cmd::retry::run(&root, &id),
Command::Kill { id } => crate::cmd::kill::run(&root, &id).await,
```

Remove `fn cmd_retry` and `async fn cmd_kill` from `main.rs`.

**Step 4: Verify + Commit**

```bash
cargo check 2>&1 | grep -E "^error" | head -10
git -C /Users/studio/gtfs add crates/jc/src/cmd/retry.rs crates/jc/src/cmd/kill.rs crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract retry and kill commands to cmd/"
```

---

## Task 14: Create cmd/logs.rs

**Files:**
- Modify: `crates/jc/src/cmd/logs.rs`
- Modify: `crates/jc/src/main.rs`

**Step 1: Write cmd/logs.rs**

Copy `cmd_logs` (lines 1390–1468) and `print_log_section` (lines 1470–1482):

```rust
// crates/jc/src/cmd/logs.rs
use anyhow::Context as _;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;
use crate::util::fs::{find_card, find_card_in_state};

pub async fn run(root: &Path, id: &str, follow: bool) -> anyhow::Result<()> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let stdout_log = card.join("logs").join("stdout.log");
    let stderr_log = card.join("logs").join("stderr.log");

    if !follow {
        print_log_section("stdout", &stdout_log)?;
        print_log_section("stderr", &stderr_log)?;
        return Ok(());
    }

    let mut stdout_file = fs::File::open(&stdout_log)
        .unwrap_or_else(|_| fs::File::open("/dev/null").expect("open /dev/null"));
    let mut stderr_file = fs::File::open(&stderr_log)
        .unwrap_or_else(|_| fs::File::open("/dev/null").expect("open /dev/null"));

    let mut buf = Vec::new();
    stdout_file.read_to_end(&mut buf)?;
    if !buf.is_empty() { print!("{}", String::from_utf8_lossy(&buf)); }
    let mut stdout_pos = stdout_file.stream_position()?;
    buf.clear();
    stderr_file.read_to_end(&mut buf)?;
    if !buf.is_empty() { eprint!("{}", String::from_utf8_lossy(&buf)); }
    let mut stderr_pos = stderr_file.stream_position()?;

    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if !stdout_log.exists() {
            if let Ok(f) = fs::File::open(&stdout_log) { stdout_file = f; stdout_pos = 0; }
        }
        if !stderr_log.exists() {
            if let Ok(f) = fs::File::open(&stderr_log) { stderr_file = f; stderr_pos = 0; }
        }
        stdout_file.seek(SeekFrom::Start(stdout_pos))?;
        buf.clear();
        stdout_file.read_to_end(&mut buf)?;
        if !buf.is_empty() {
            print!("{}", String::from_utf8_lossy(&buf));
            std::io::stdout().flush()?;
            stdout_pos += buf.len() as u64;
        }
        stderr_file.seek(SeekFrom::Start(stderr_pos))?;
        buf.clear();
        stderr_file.read_to_end(&mut buf)?;
        if !buf.is_empty() {
            eprint!("{}", String::from_utf8_lossy(&buf));
            std::io::stderr().flush()?;
            stderr_pos += buf.len() as u64;
        }
        if !find_card_in_state(root, id, "running") { break; }
    }
    Ok(())
}

fn print_log_section(label: &str, path: &Path) -> anyhow::Result<()> {
    if !path.exists() { println!("=== {} (no file) ===", label); return Ok(()); }
    let content = fs::read_to_string(path)?;
    println!("=== {} ===", label);
    print!("{}", content);
    if !content.ends_with('\n') && !content.is_empty() { println!(); }
    Ok(())
}
```

**Step 2: Update main.rs**

```rust
Command::Logs { id, follow } => crate::cmd::logs::run(&root, &id, follow).await,
```

Remove `async fn cmd_logs` and `fn print_log_section` and `fn find_card_in_state` from `main.rs`.

**Step 3: Verify + Commit**

```bash
cargo check 2>&1 | grep -E "^error" | head -10
git -C /Users/studio/gtfs add crates/jc/src/cmd/logs.rs crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract logs command to cmd/logs.rs"
```

---

## Task 15: Create cmd/inspect.rs

**Files:**
- Modify: `crates/jc/src/cmd/inspect.rs`
- Modify: `crates/jc/src/main.rs`

**Step 1: Write cmd/inspect.rs**

Copy `cmd_inspect` (lines 1490–1530):

```rust
// crates/jc/src/cmd/inspect.rs
use anyhow::Context as _;
use std::fs;
use std::path::Path;
use crate::util::fs::find_card;

pub fn run(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let state = card.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap_or("unknown");
    println!("=== meta ({}) ===", state);
    let meta = jobcard_core::read_meta(&card)?;
    println!("{}", serde_json::to_string_pretty(&meta)?);
    let spec_path = card.join("spec.md");
    if spec_path.exists() {
        let spec = fs::read_to_string(&spec_path)?;
        println!("\n=== spec.md ===");
        print!("{}", spec);
        if !spec.ends_with('\n') && !spec.is_empty() { println!(); }
    }
    for (label, filename) in [("stdout", "stdout.log"), ("stderr", "stderr.log")] {
        let log_path = card.join("logs").join(filename);
        if log_path.exists() {
            let content = fs::read_to_string(&log_path)?;
            let lines: Vec<&str> = content.lines().collect();
            let tail_lines = if lines.len() > 20 { &lines[lines.len() - 20..] } else { &lines[..] };
            println!("\n=== {} (last {} lines) ===", label, tail_lines.len());
            for line in tail_lines { println!("{}", line); }
        }
    }
    Ok(())
}
```

**Step 2: Update main.rs**

```rust
Command::Inspect { id } => crate::cmd::inspect::run(&root, &id),
```

Remove `fn cmd_inspect` from `main.rs`.

**Step 3: Verify + Commit**

```bash
cargo check 2>&1 | grep -E "^error" | head -10
git -C /Users/studio/gtfs add crates/jc/src/cmd/inspect.rs crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract inspect command to cmd/inspect.rs"
```

---

## Task 16: Create cmd/memory.rs

**Files:**
- Modify: `crates/jc/src/cmd/memory.rs`
- Modify: `crates/jc/src/main.rs`

**Step 1: Write cmd/memory.rs**

Copy the four `cmd_memory_*` functions (lines 609–678):

```rust
// crates/jc/src/cmd/memory.rs
use anyhow::Context as _;
use chrono::Utc;
use std::path::Path;
use crate::MemoryCommand;
use crate::util::memory::{
    normalize_namespace, read_memory_store, set_memory_entry, write_memory_store,
    DEFAULT_MEMORY_TTL_SECONDS,
};

pub fn run(root: &Path, cmd: MemoryCommand) -> anyhow::Result<()> {
    match cmd {
        MemoryCommand::List { namespace } => list(root, &namespace),
        MemoryCommand::Get { namespace, key } => get(root, &namespace, &key),
        MemoryCommand::Set { namespace, key, value, ttl_seconds } => {
            set(root, &namespace, &key, &value, ttl_seconds)
        }
        MemoryCommand::Delete { namespace, key } => delete(root, &namespace, &key),
    }
}

fn list(root: &Path, namespace: &str) -> anyhow::Result<()> {
    let namespace = normalize_namespace(namespace);
    let store = read_memory_store(root, &namespace)?;
    if store.entries.is_empty() { println!("(empty)"); return Ok(()); }
    for (key, entry) in store.entries {
        let expires = entry.expires_at.map(|t| t.to_rfc3339()).unwrap_or_else(|| "never".to_string());
        println!("{}\t{}\t{}", key, entry.value, expires);
    }
    Ok(())
}

fn get(root: &Path, namespace: &str, key: &str) -> anyhow::Result<()> {
    let namespace = normalize_namespace(namespace);
    let key = key.trim();
    if key.is_empty() { anyhow::bail!("key cannot be empty"); }
    let store = read_memory_store(root, &namespace)?;
    let entry = store.entries.get(key).with_context(|| format!("memory key not found: {}", key))?;
    println!("{}", entry.value);
    Ok(())
}

fn set(root: &Path, namespace: &str, key: &str, value: &str, ttl_seconds: i64) -> anyhow::Result<()> {
    if ttl_seconds <= 0 { anyhow::bail!("ttl_seconds must be > 0"); }
    let namespace = normalize_namespace(namespace);
    let key = key.trim();
    if key.is_empty() { anyhow::bail!("key cannot be empty"); }
    let mut store = read_memory_store(root, &namespace)?;
    set_memory_entry(&mut store, key, value, ttl_seconds, Utc::now());
    write_memory_store(root, &namespace, &store)?;
    Ok(())
}

fn delete(root: &Path, namespace: &str, key: &str) -> anyhow::Result<()> {
    let namespace = normalize_namespace(namespace);
    let key = key.trim();
    if key.is_empty() { anyhow::bail!("key cannot be empty"); }
    let mut store = read_memory_store(root, &namespace)?;
    store.entries.remove(key);
    write_memory_store(root, &namespace, &store)?;
    Ok(())
}
```

**Step 2: Update main.rs**

```rust
Command::Memory { cmd } => crate::cmd::memory::run(&root, cmd),
```

Remove the four `cmd_memory_*` functions from `main.rs`.

**Step 3: Verify + Commit**

```bash
cargo check 2>&1 | grep -E "^error" | head -10
git -C /Users/studio/gtfs add crates/jc/src/cmd/memory.rs crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: extract memory command to cmd/memory.rs"
```

---

## Task 17: Slim down main.rs

At this point, all command logic has moved out. `main.rs` should only contain:
- `use` statements (just what's needed for clap types and `main()`)
- `Cli`, `Command`, `MemoryCommand` enums
- `#[tokio::main] async fn main()`

**Files:**
- Modify: `crates/jc/src/main.rs`

**Step 1: Remove all now-unused imports**

The only imports `main.rs` needs are:
```rust
use clap::{Parser, Subcommand};
use std::path::PathBuf;
```
(Plus `mod util;` and `mod cmd;`)

**Step 2: Verify main.rs is ≤ 300 lines**

```bash
wc -l /Users/studio/gtfs/crates/jc/src/main.rs
```

Expected: ≤ 100 lines.

**Step 3: Verify all files are under 300 lines**

```bash
find /Users/studio/gtfs/crates/jc/src -name "*.rs" | xargs wc -l | sort -rn | head -20
```

Expected: no file exceeds 300 lines.

**Step 4: Commit**

```bash
git -C /Users/studio/gtfs add crates/jc/src/main.rs
git -C /Users/studio/gtfs commit -m "refactor: slim main.rs to CLI types and dispatcher only"
```

---

## Task 18: Full Verification

**Step 1: Build**

```bash
cd /Users/studio/gtfs && cargo build 2>&1 | tail -10
```

Expected: `Finished dev` with no errors.

**Step 2: Run all tests**

```bash
cd /Users/studio/gtfs && cargo test 2>&1 | tail -30
```

Expected: all integration harness tests pass. Specifically:
- `dispatcher_moves_success_to_done` ✓
- `dispatcher_rate_limit_requeues_to_pending` ✓
- `dispatcher_rate_limit_sets_cooldown_and_rotates_chain` ✓
- `dispatcher_relative_adapter_path_works` ✓
- `dispatcher_qa_prefers_different_provider_than_implement` ✓
- All merge_gate_harness and job_control_harness tests ✓

**Step 3: Lint**

```bash
cd /Users/studio/gtfs && cargo clippy -- -D warnings 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

**Step 4: Format check**

```bash
cd /Users/studio/gtfs && cargo fmt --check 2>&1
```

If there are formatting issues, run `cargo fmt` then commit.

**Step 5: Final commit**

```bash
git -C /Users/studio/gtfs add -A
git -C /Users/studio/gtfs commit -m "chore: cargo fmt after module refactoring"
```

---

## Acceptance Criteria Checklist

- [ ] Each CLI command has its own module file under `crates/jc/src/cmd/`
- [ ] Shared filesystem and meta utilities are extracted to `crates/jc/src/util/`
- [ ] All existing tests pass (`cargo test`)
- [ ] No single source file exceeds 300 lines (`find ... | xargs wc -l`)
- [ ] Stub crates removed (`crates/jc-dispatcher/` and `crates/jc-merge-gate/` deleted)
- [ ] `cargo clippy -- -D warnings` is clean
- [ ] `cargo fmt --check` is clean
