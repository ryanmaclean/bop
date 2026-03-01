# Providers CLI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `jc providers` subcommands (list, add, remove, status) so engineers can manage providers.json without manually editing JSON.

**Architecture:** All changes land in `crates/jc/src/main.rs`. The `Provider` struct gains a `model` field. A `Providers` top-level subcommand branches into four sub-subcommands. Validation is added to `read_providers`/`write_providers`. Stats for `status` are computed by scanning all card state directories and tallying `meta.stages[*].provider`.

**Tech Stack:** Rust, Clap 4 (derive), Serde/serde_json, chrono (timestamps), anyhow (error handling), std::io (stdin confirmation prompt).

---

### Task 1: Add `model` field to `Provider` + validation

**Files:**
- Modify: `crates/jc/src/main.rs:146-178`
- Test: `crates/jc/tests/providers_harness.rs` (new file)

**Step 1: Write the failing test**

Create `crates/jc/tests/providers_harness.rs` with:

```rust
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().to_path_buf()
}

fn build_jc() {
    let status = Command::new("cargo")
        .arg("build")
        .current_dir(repo_root())
        .status()
        .expect("cargo build failed to start");
    assert!(status.success());
}

fn jc_bin() -> PathBuf {
    repo_root().join("target").join("debug").join("jc")
}

fn init_cards(td: &Path) -> PathBuf {
    let cards = td.join(".cards");
    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());
    cards
}

#[test]
fn providers_list_shows_seeded_providers() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "providers", "list"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("mock"), "expected 'mock' in: {}", stdout);
    assert!(stdout.contains("mock2"), "expected 'mock2' in: {}", stdout);
}
```

**Step 2: Run to verify it fails**

```
cargo test -p jc providers_list_shows_seeded_providers 2>&1 | head -30
```
Expected: compile error — `providers` subcommand doesn't exist yet.

**Step 3: Add `model` field to `Provider`**

In `crates/jc/src/main.rs`, change the `Provider` struct (line 146–153):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Provider {
    command: String,
    #[serde(default)]
    rate_limit_exit: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cooldown_until_epoch_s: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}
```

Add a validation function after the struct (before `providers_path`):

```rust
fn validate_provider(name: &str, p: &Provider) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("provider name cannot be empty");
    }
    if p.command.trim().is_empty() {
        anyhow::bail!("provider '{}': command/adapter cannot be empty", name);
    }
    Ok(())
}
```

Update `read_providers` to validate on read:

```rust
fn read_providers(cards_dir: &Path) -> anyhow::Result<ProvidersFile> {
    let p = providers_path(cards_dir);
    if !p.exists() {
        return Ok(ProvidersFile::default());
    }
    let bytes = fs::read(p)?;
    let pf: ProvidersFile = serde_json::from_slice(&bytes)?;
    for (name, provider) in &pf.providers {
        validate_provider(name, provider)?;
    }
    Ok(pf)
}
```

Update `write_providers` to validate on write:

```rust
fn write_providers(cards_dir: &Path, pf: &ProvidersFile) -> anyhow::Result<()> {
    for (name, provider) in &pf.providers {
        validate_provider(name, provider)?;
    }
    let bytes = serde_json::to_vec_pretty(pf)?;
    fs::write(providers_path(cards_dir), bytes)?;
    Ok(())
}
```

**Step 4: Build to verify no compile errors**

```
cargo build -p jc 2>&1
```
Expected: `Finished` with no errors.

**Step 5: Commit**

```bash
git add crates/jc/src/main.rs crates/jc/tests/providers_harness.rs
git commit -m "feat: add model field to Provider + read/write validation"
```

---

### Task 2: Add `Providers` subcommand skeleton to CLI

**Files:**
- Modify: `crates/jc/src/main.rs:24-67` (Command enum)

**Step 1: Add `ProvidersCommand` enum and `Providers` variant**

In `main.rs`, after the `Command` enum definition, add:

```rust
#[derive(Subcommand, Debug)]
enum ProvidersCommand {
    List,
    Add {
        name: String,
        #[arg(long)]
        adapter: String,
        #[arg(long)]
        model: Option<String>,
    },
    Remove {
        name: String,
        #[arg(long)]
        force: bool,
    },
    Status,
}
```

Add `Providers` variant to the existing `Command` enum:

```rust
Providers {
    #[command(subcommand)]
    cmd: ProvidersCommand,
},
```

**Step 2: Add stub match arm in `main()`**

In the `match cli.cmd { ... }` block (around line 343), add a stub before the closing brace:

```rust
Command::Providers { cmd } => run_providers(&root, cmd),
```

Add a stub function after the existing helper functions:

```rust
fn run_providers(cards_dir: &Path, cmd: ProvidersCommand) -> anyhow::Result<()> {
    match cmd {
        ProvidersCommand::List => todo!("providers list"),
        ProvidersCommand::Add { .. } => todo!("providers add"),
        ProvidersCommand::Remove { .. } => todo!("providers remove"),
        ProvidersCommand::Status => todo!("providers status"),
    }
}
```

**Step 3: Build to verify it compiles**

```
cargo build -p jc 2>&1
```
Expected: `Finished` (todo! panics are not compile errors).

**Step 4: Commit**

```bash
git add crates/jc/src/main.rs
git commit -m "feat: add Providers subcommand skeleton to CLI"
```

---

### Task 3: Implement `providers list`

**Files:**
- Modify: `crates/jc/src/main.rs` — `run_providers` function
- Test: `crates/jc/tests/providers_harness.rs`

**Step 1: Write the failing test**

Add to `providers_harness.rs`:

```rust
#[test]
fn providers_list_shows_all_fields() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    // Manually write a provider with a model and cooldown
    let now_plus_300 = chrono::Utc::now().timestamp() + 300;
    let json = format!(
        r#"{{"providers":{{"cool-provider":{{"command":"adapters/mock.sh","rate_limit_exit":75,"cooldown_until_epoch_s":{},"model":"gpt-4o"}},"no-cool":{{"command":"adapters/mock.sh","rate_limit_exit":75}}}}}}"#,
        now_plus_300
    );
    fs::write(cards.join("providers.json"), json).unwrap();

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "providers", "list"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // cool-provider should show a cooldown remaining > 0
    assert!(stdout.contains("cool-provider"), "{}", stdout);
    assert!(stdout.contains("gpt-4o"), "{}", stdout);
    // Should show remaining seconds (approximately 300)
    assert!(stdout.contains("cooldown"), "{}", stdout);
    // no-cool should show no cooldown
    assert!(stdout.contains("no-cool"), "{}", stdout);
}
```

**Step 2: Run to verify it fails**

```
cargo test -p jc providers_list_shows_all_fields 2>&1 | tail -20
```
Expected: panics with `todo!("providers list")`.

**Step 3: Implement `providers list`**

Replace the `ProvidersCommand::List => todo!(...)` arm in `run_providers`:

```rust
ProvidersCommand::List => {
    let pf = read_providers(cards_dir)?;
    let now = chrono::Utc::now().timestamp();
    if pf.providers.is_empty() {
        println!("No providers configured. Run: jc providers add <name> --adapter <script>");
        return Ok(());
    }
    println!("{:<20} {:<30} {:<20} {}", "NAME", "ADAPTER", "MODEL", "COOLDOWN");
    for (name, p) in &pf.providers {
        let model = p.model.as_deref().unwrap_or("-");
        let cooldown = match p.cooldown_until_epoch_s {
            Some(until) if until > now => format!("{}s remaining", until - now),
            _ => "none".to_string(),
        };
        println!("{:<20} {:<30} {:<20} {}", name, p.command, model, cooldown);
    }
    Ok(())
}
```

**Step 4: Run test to verify it passes**

```
cargo test -p jc providers_list 2>&1
```
Expected: all `providers_list` tests pass.

**Step 5: Commit**

```bash
git add crates/jc/src/main.rs crates/jc/tests/providers_harness.rs
git commit -m "feat: implement providers list command"
```

---

### Task 4: Implement `providers add`

**Files:**
- Modify: `crates/jc/src/main.rs`
- Test: `crates/jc/tests/providers_harness.rs`

**Step 1: Write the failing tests**

Add to `providers_harness.rs`:

```rust
#[test]
fn providers_add_creates_provider() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let mock_adapter = repo_root().join("adapters").join("mock.sh");

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir", cards.to_str().unwrap(),
            "providers", "add", "claude-3",
            "--adapter", mock_adapter.to_str().unwrap(),
            "--model", "claude-opus-4-6",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let json = fs::read_to_string(cards.join("providers.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v["providers"]["claude-3"]["command"].as_str().is_some());
    assert_eq!(v["providers"]["claude-3"]["model"].as_str(), Some("claude-opus-4-6"));
}

#[test]
fn providers_add_rejects_duplicate_name() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let mock_adapter = repo_root().join("adapters").join("mock.sh");

    // Add once — should succeed
    let status = Command::new(jc_bin())
        .args([
            "--cards-dir", cards.to_str().unwrap(),
            "providers", "add", "my-prov",
            "--adapter", mock_adapter.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    // Add again — should fail
    let status = Command::new(jc_bin())
        .args([
            "--cards-dir", cards.to_str().unwrap(),
            "providers", "add", "my-prov",
            "--adapter", mock_adapter.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(!status.success(), "duplicate add should fail");
}

#[test]
fn providers_add_rejects_empty_adapter() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let out = Command::new(jc_bin())
        .args([
            "--cards-dir", cards.to_str().unwrap(),
            "providers", "add", "bad-prov",
            "--adapter", "",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
}
```

**Step 2: Run to verify they fail**

```
cargo test -p jc providers_add 2>&1 | tail -30
```
Expected: panics with `todo!("providers add")`.

**Step 3: Implement `providers add`**

Replace the `ProvidersCommand::Add { .. } => todo!(...)` arm:

```rust
ProvidersCommand::Add { name, adapter, model } => {
    // Validate inputs
    if name.trim().is_empty() {
        anyhow::bail!("provider name cannot be empty");
    }
    if adapter.trim().is_empty() {
        anyhow::bail!("--adapter cannot be empty");
    }

    let mut pf = read_providers(cards_dir)?;

    if pf.providers.contains_key(&name) {
        anyhow::bail!("provider '{}' already exists. Use 'providers remove' first.", name);
    }

    let provider = Provider {
        command: adapter,
        rate_limit_exit: 75,
        cooldown_until_epoch_s: None,
        model,
    };
    validate_provider(&name, &provider)?;

    pf.providers.insert(name.clone(), provider);
    write_providers(cards_dir, &pf)?;
    println!("Added provider '{}'.", name);
    Ok(())
}
```

**Step 4: Run tests to verify they pass**

```
cargo test -p jc providers_add 2>&1
```
Expected: all `providers_add` tests pass.

**Step 5: Commit**

```bash
git add crates/jc/src/main.rs crates/jc/tests/providers_harness.rs
git commit -m "feat: implement providers add command"
```

---

### Task 5: Implement `providers remove`

**Files:**
- Modify: `crates/jc/src/main.rs`
- Test: `crates/jc/tests/providers_harness.rs`

**Step 1: Write the failing tests**

Add to `providers_harness.rs`:

```rust
fn write_running_card_with_provider(cards: &Path, card_id: &str, provider: &str) {
    let card_dir = cards.join("running").join(format!("{}.jobcard", card_id));
    fs::create_dir_all(card_dir.join("logs")).unwrap();
    fs::create_dir_all(card_dir.join("output")).unwrap();
    let meta = format!(
        r#"{{"id":"{id}","created":"2026-03-01T00:00:00Z","stage":"implement","stages":{{"implement":{{"status":"running","provider":"{prov}"}}}},"acceptance_criteria":[]}}"#,
        id = card_id, prov = provider
    );
    fs::write(card_dir.join("meta.json"), meta).unwrap();
}

#[test]
fn providers_remove_deletes_provider() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir", cards.to_str().unwrap(),
            "providers", "remove", "mock2",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let json = fs::read_to_string(cards.join("providers.json")).unwrap();
    assert!(!json.contains("\"mock2\""), "mock2 should be removed");
    assert!(json.contains("\"mock\""), "mock should remain");
}

#[test]
fn providers_remove_blocks_when_provider_has_active_jobs() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    // Place a running card using 'mock'
    write_running_card_with_provider(&cards, "active-job", "mock");

    let out = Command::new(jc_bin())
        .args([
            "--cards-dir", cards.to_str().unwrap(),
            "providers", "remove", "mock",
        ])
        .output()
        .unwrap();
    // Should fail without --force
    assert!(!out.status.success(), "should fail when active jobs exist");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("active") || stderr.contains("force"),
        "error should mention active jobs or --force: {}", stderr
    );
}

#[test]
fn providers_remove_force_removes_despite_active_jobs() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    write_running_card_with_provider(&cards, "active-job2", "mock");

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir", cards.to_str().unwrap(),
            "providers", "remove", "mock", "--force",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let json = fs::read_to_string(cards.join("providers.json")).unwrap();
    assert!(!json.contains("\"mock\""), "mock should be removed");
}

#[test]
fn providers_remove_nonexistent_errors() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let out = Command::new(jc_bin())
        .args([
            "--cards-dir", cards.to_str().unwrap(),
            "providers", "remove", "does-not-exist",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
}
```

**Step 2: Run to verify they fail**

```
cargo test -p jc providers_remove 2>&1 | tail -30
```
Expected: panics with `todo!("providers remove")`.

**Step 3: Add helper to count active jobs for a provider**

Add this function near the other provider helpers in `main.rs`:

```rust
fn count_active_jobs_for_provider(cards_dir: &Path, provider_name: &str) -> usize {
    let running_dir = cards_dir.join("running");
    let Ok(entries) = std::fs::read_dir(&running_dir) else { return 0 };
    let mut count = 0;
    for ent in entries.flatten() {
        let card_dir = ent.path();
        if !card_dir.is_dir() { continue; }
        if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" { continue; }
        if let Ok(meta) = jobcard_core::read_meta(&card_dir) {
            for record in meta.stages.values() {
                if record.provider.as_deref() == Some(provider_name) {
                    count += 1;
                    break;
                }
            }
        }
    }
    count
}
```

**Step 4: Implement `providers remove`**

Replace the `ProvidersCommand::Remove { .. } => todo!(...)` arm:

```rust
ProvidersCommand::Remove { name, force } => {
    let mut pf = read_providers(cards_dir)?;
    if !pf.providers.contains_key(&name) {
        anyhow::bail!("provider '{}' not found", name);
    }

    let active = count_active_jobs_for_provider(cards_dir, &name);
    if active > 0 && !force {
        anyhow::bail!(
            "provider '{}' has {} active job(s) in running/. \
             Use --force to remove anyway.",
            name, active
        );
    }

    pf.providers.remove(&name);
    write_providers(cards_dir, &pf)?;
    if active > 0 {
        eprintln!("Warning: removed '{}' with {} active job(s).", name, active);
    }
    println!("Removed provider '{}'.", name);
    Ok(())
}
```

**Step 5: Run tests to verify they pass**

```
cargo test -p jc providers_remove 2>&1
```
Expected: all `providers_remove` tests pass.

**Step 6: Commit**

```bash
git add crates/jc/src/main.rs crates/jc/tests/providers_harness.rs
git commit -m "feat: implement providers remove command"
```

---

### Task 6: Implement `providers status`

**Files:**
- Modify: `crates/jc/src/main.rs`
- Test: `crates/jc/tests/providers_harness.rs`

**Step 1: Write the failing tests**

Add to `providers_harness.rs`:

```rust
fn write_done_card_with_provider(cards: &Path, card_id: &str, provider: &str, success: bool) {
    let state = if success { "done" } else { "failed" };
    let card_dir = cards.join(state).join(format!("{}.jobcard", card_id));
    fs::create_dir_all(card_dir.join("logs")).unwrap();
    fs::create_dir_all(card_dir.join("output")).unwrap();
    let stage_status = if success { "done" } else { "failed" };
    let meta = format!(
        r#"{{"id":"{id}","created":"2026-03-01T00:00:00Z","stage":"implement","stages":{{"implement":{{"status":"{ss}","provider":"{prov}"}}}},"acceptance_criteria":[]}}"#,
        id = card_id, ss = stage_status, prov = provider
    );
    fs::write(card_dir.join("meta.json"), meta).unwrap();
}

#[test]
fn providers_status_shows_job_counts() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    // 2 successes and 1 failure for mock
    write_done_card_with_provider(&cards, "j1", "mock", true);
    write_done_card_with_provider(&cards, "j2", "mock", true);
    write_done_card_with_provider(&cards, "j3", "mock", false);
    // 1 success for mock2
    write_done_card_with_provider(&cards, "j4", "mock2", true);

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "providers", "status"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);

    // mock: 3 total, 2 success
    assert!(stdout.contains("mock"), "{}", stdout);
    // The output should contain job count numbers
    assert!(stdout.contains('3') || stdout.contains('2'), "{}", stdout);
}

#[test]
fn providers_status_shows_cooldown() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let now_plus_300 = chrono::Utc::now().timestamp() + 300;
    let json = format!(
        r#"{{"providers":{{"cool-prov":{{"command":"adapters/mock.sh","rate_limit_exit":75,"cooldown_until_epoch_s":{}}}}}}}"#,
        now_plus_300
    );
    fs::write(cards.join("providers.json"), json).unwrap();

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "providers", "status"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cool-prov"), "{}", stdout);
    assert!(stdout.contains("cooldown") || stdout.contains("300") || stdout.contains("29"), "{}", stdout);
}
```

**Step 2: Run to verify they fail**

```
cargo test -p jc providers_status 2>&1 | tail -20
```
Expected: panics with `todo!("providers status")`.

**Step 3: Add helper to compute per-provider stats**

Add this function to `main.rs`:

```rust
struct ProviderStats {
    total: usize,
    success: usize,
    failed: usize,
}

fn compute_provider_stats(cards_dir: &Path) -> std::collections::BTreeMap<String, ProviderStats> {
    let mut stats: std::collections::BTreeMap<String, ProviderStats> = Default::default();
    let state_dirs = ["pending", "running", "done", "merged", "failed"];
    for state in state_dirs {
        let dir = cards_dir.join(state);
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for ent in entries.flatten() {
            let card_dir = ent.path();
            if !card_dir.is_dir() { continue; }
            if card_dir.extension().and_then(|s| s.to_str()).unwrap_or("") != "jobcard" { continue; }
            let Ok(meta) = jobcard_core::read_meta(&card_dir) else { continue };
            for record in meta.stages.values() {
                let Some(prov) = &record.provider else { continue };
                let entry = stats.entry(prov.clone()).or_insert(ProviderStats {
                    total: 0, success: 0, failed: 0
                });
                entry.total += 1;
                match record.status {
                    jobcard_core::StageStatus::Done => entry.success += 1,
                    jobcard_core::StageStatus::Failed => entry.failed += 1,
                    _ => {}
                }
            }
        }
    }
    stats
}
```

**Step 4: Implement `providers status`**

Replace the `ProvidersCommand::Status => todo!(...)` arm:

```rust
ProvidersCommand::Status => {
    let pf = read_providers(cards_dir)?;
    let stats = compute_provider_stats(cards_dir);
    let now = chrono::Utc::now().timestamp();

    if pf.providers.is_empty() {
        println!("No providers configured.");
        return Ok(());
    }

    println!("{:<20} {:>6} {:>8} {:>8} {}", "PROVIDER", "TOTAL", "SUCCESS", "FAILED", "COOLDOWN");
    for (name, p) in &pf.providers {
        let s = stats.get(name);
        let total = s.map(|s| s.total).unwrap_or(0);
        let success = s.map(|s| s.success).unwrap_or(0);
        let failed = s.map(|s| s.failed).unwrap_or(0);
        let cooldown = match p.cooldown_until_epoch_s {
            Some(until) if until > now => format!("{}s", until - now),
            _ => "none".to_string(),
        };
        println!("{:<20} {:>6} {:>8} {:>8} {}", name, total, success, failed, cooldown);
    }
    Ok(())
}
```

**Step 5: Run all provider tests**

```
cargo test -p jc providers 2>&1
```
Expected: all `providers_*` tests pass.

**Step 6: Run full test suite**

```
cargo test 2>&1
```
Expected: all tests pass.

**Step 7: Run linting and format check**

```
cargo clippy -- -D warnings 2>&1
cargo fmt --check 2>&1
```
Fix any warnings, then:
```
cargo fmt 2>&1
```

**Step 8: Commit**

```bash
git add crates/jc/src/main.rs crates/jc/tests/providers_harness.rs
git commit -m "feat: implement providers status command"
```

---

### Task 7: Final verification

**Step 1: Run make check**

```
make check 2>&1
```
Expected: `test + lint + fmt` all pass with no errors.

**Step 2: Smoke test the binary manually**

```bash
# From repo root, using a temp dir
TMPDIR=$(mktemp -d) && ./target/debug/jc --cards-dir "$TMPDIR/.cards" init

# List shows seeded mock/mock2
./target/debug/jc --cards-dir "$TMPDIR/.cards" providers list

# Add a new provider
./target/debug/jc --cards-dir "$TMPDIR/.cards" providers add my-claude \
  --adapter adapters/mock.sh --model claude-opus-4-6

# List again — should show my-claude
./target/debug/jc --cards-dir "$TMPDIR/.cards" providers list

# Status (no jobs yet)
./target/debug/jc --cards-dir "$TMPDIR/.cards" providers status

# Remove a provider
./target/debug/jc --cards-dir "$TMPDIR/.cards" providers remove mock2

# List — mock2 should be gone
./target/debug/jc --cards-dir "$TMPDIR/.cards" providers list
```

**Step 3: Commit final cleanup if needed**

```bash
git add -u
git commit -m "chore: cleanup after providers-cli smoke test"
```
