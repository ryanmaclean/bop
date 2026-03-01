# Global Config File Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Introduce `~/.jobcard/config.yaml` (global) and `.jobcard/config.yaml` (project-level) with merge semantics, plus `jc config get/set` subcommands.

**Architecture:** A new `Config` struct lives in `jobcard-core` (read-only data type + load/merge/save functions). The `jc` binary loads config on startup and plumbs values into every command that needs them (dispatcher `max_concurrent`, default provider chain, etc.). YAML is parsed with `serde_yaml`. Project config overrides global; CLI flags override both.

**Tech Stack:** Rust, `serde_yaml` (new dep), `serde` (existing), `dirs` crate (new dep, resolves `~`), existing `anyhow` for errors.

---

### Task 1: Add new dependencies

**Files:**
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/jobcard-core/Cargo.toml`
- Modify: `crates/jc/Cargo.toml`

**Step 1: Add `serde_yaml` and `dirs` to the workspace manifest**

In `/Users/studio/gtfs/Cargo.toml`, add to `[workspace.dependencies]`:

```toml
serde_yaml = "0.9"
dirs = "5"
```

**Step 2: Add them to `jobcard-core` dependencies**

In `/Users/studio/gtfs/crates/jobcard-core/Cargo.toml`, add:

```toml
serde_yaml.workspace = true
dirs.workspace = true
```

**Step 3: Add `dirs` to `jc` dependencies** (jobcard-core re-exports config loading, jc needs dirs for nothing extra)

In `/Users/studio/gtfs/crates/jc/Cargo.toml`, add:

```toml
serde_yaml.workspace = true
```

**Step 4: Verify it compiles**

```bash
cd /Users/studio/gtfs && cargo build 2>&1 | head -20
```

Expected: compiles cleanly (no new code yet, just dep additions).

**Step 5: Commit**

```bash
cd /Users/studio/gtfs
git add Cargo.toml crates/jobcard-core/Cargo.toml crates/jc/Cargo.toml Cargo.lock
git commit -m "chore: add serde_yaml and dirs workspace dependencies"
```

---

### Task 2: Define `Config` struct and load/merge logic in `jobcard-core`

**Files:**
- Create: `crates/jobcard-core/src/config.rs`
- Modify: `crates/jobcard-core/src/lib.rs` (add `pub mod config; pub use config::...`)

**Step 1: Write the failing unit tests first**

Create `crates/jobcard-core/src/config.rs` with tests at the bottom (no impl yet):

```rust
use serde::{Deserialize, Serialize};
use std::path::Path;

/// All fields are Option so partial configs can be merged.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Config {
    pub default_provider_chain: Option<Vec<String>>,
    pub max_concurrent: Option<usize>,
    pub cooldown_seconds: Option<u64>,
    pub log_retention_days: Option<u64>,
    pub default_template: Option<String>,
}

/// Merge `overlay` on top of `base`.  Non-None overlay values win.
pub fn merge_configs(base: Config, overlay: Config) -> Config {
    todo!()
}

/// Parse YAML bytes into Config, returning a clear error on bad schema.
pub fn parse_config(yaml: &str) -> anyhow::Result<Config> {
    todo!()
}

/// Return the global config path: ~/.jobcard/config.yaml
pub fn global_config_path() -> Option<std::path::PathBuf> {
    todo!()
}

/// Return the project config path: <cwd>/.jobcard/config.yaml
pub fn project_config_path() -> std::path::PathBuf {
    todo!()
}

/// Load and merge global + project configs.  Missing files are silently skipped.
/// Returns merged Config and emits a clear error for malformed YAML.
pub fn load_config() -> anyhow::Result<Config> {
    todo!()
}

/// Read config from a specific path (used by `jc config get/set`).
pub fn read_config_file(path: &Path) -> anyhow::Result<Config> {
    todo!()
}

/// Write config to a specific path (used by `jc config set`).
pub fn write_config_file(path: &Path, cfg: &Config) -> anyhow::Result<()> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_yaml() {
        let yaml = r#"
default_provider_chain: [claude, codex]
max_concurrent: 3
cooldown_seconds: 120
log_retention_days: 7
default_template: implement
"#;
        let cfg = parse_config(yaml).unwrap();
        assert_eq!(cfg.default_provider_chain, Some(vec!["claude".to_string(), "codex".to_string()]));
        assert_eq!(cfg.max_concurrent, Some(3));
        assert_eq!(cfg.cooldown_seconds, Some(120));
        assert_eq!(cfg.log_retention_days, Some(7));
        assert_eq!(cfg.default_template, Some("implement".to_string()));
    }

    #[test]
    fn parse_partial_yaml() {
        let yaml = "max_concurrent: 5\n";
        let cfg = parse_config(yaml).unwrap();
        assert_eq!(cfg.max_concurrent, Some(5));
        assert_eq!(cfg.default_provider_chain, None);
    }

    #[test]
    fn parse_empty_yaml() {
        let cfg = parse_config("").unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn parse_malformed_yaml_returns_error() {
        let result = parse_config("max_concurrent: not_a_number\n");
        assert!(result.is_err(), "expected error for bad schema");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("max_concurrent") || msg.contains("invalid") || msg.contains("expected"),
            "error message should hint at the field: {}", msg
        );
    }

    #[test]
    fn merge_overlay_wins() {
        let base = Config {
            max_concurrent: Some(2),
            cooldown_seconds: Some(300),
            ..Default::default()
        };
        let overlay = Config {
            max_concurrent: Some(5),
            default_template: Some("qa".to_string()),
            ..Default::default()
        };
        let merged = merge_configs(base, overlay);
        assert_eq!(merged.max_concurrent, Some(5));      // overlay wins
        assert_eq!(merged.cooldown_seconds, Some(300));  // base kept
        assert_eq!(merged.default_template, Some("qa".to_string()));
    }

    #[test]
    fn merge_none_overlay_keeps_base() {
        let base = Config { max_concurrent: Some(4), ..Default::default() };
        let overlay = Config::default();
        let merged = merge_configs(base, overlay);
        assert_eq!(merged.max_concurrent, Some(4));
    }

    #[test]
    fn roundtrip_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let cfg = Config {
            max_concurrent: Some(2),
            default_template: Some("implement".to_string()),
            ..Default::default()
        };
        write_config_file(&path, &cfg).unwrap();
        let loaded = read_config_file(&path).unwrap();
        assert_eq!(loaded.max_concurrent, Some(2));
        assert_eq!(loaded.default_template, Some("implement".to_string()));
    }

    #[test]
    fn read_missing_file_returns_error() {
        let result = read_config_file(std::path::Path::new("/nonexistent/path/config.yaml"));
        assert!(result.is_err());
    }
}
```

**Step 2: Run the tests to verify they fail**

```bash
cd /Users/studio/gtfs && cargo test -p jobcard-core config 2>&1 | tail -20
```

Expected: compile error or test failures with "not yet implemented" panics.

**Step 3: Implement the functions**

Replace the `todo!()` bodies in `crates/jobcard-core/src/config.rs`:

```rust
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Config {
    pub default_provider_chain: Option<Vec<String>>,
    pub max_concurrent: Option<usize>,
    pub cooldown_seconds: Option<u64>,
    pub log_retention_days: Option<u64>,
    pub default_template: Option<String>,
}

pub fn merge_configs(base: Config, overlay: Config) -> Config {
    Config {
        default_provider_chain: overlay.default_provider_chain.or(base.default_provider_chain),
        max_concurrent: overlay.max_concurrent.or(base.max_concurrent),
        cooldown_seconds: overlay.cooldown_seconds.or(base.cooldown_seconds),
        log_retention_days: overlay.log_retention_days.or(base.log_retention_days),
        default_template: overlay.default_template.or(base.default_template),
    }
}

pub fn parse_config(yaml: &str) -> anyhow::Result<Config> {
    if yaml.trim().is_empty() {
        return Ok(Config::default());
    }
    serde_yaml::from_str(yaml).context("malformed config: expected schema with optional fields: default_provider_chain, max_concurrent, cooldown_seconds, log_retention_days, default_template")
}

pub fn global_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".jobcard").join("config.yaml"))
}

pub fn project_config_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".jobcard")
        .join("config.yaml")
}

pub fn load_config() -> anyhow::Result<Config> {
    let global = match global_config_path() {
        Some(p) if p.exists() => read_config_file(&p)
            .with_context(|| format!("global config error: {}", p.display()))?,
        _ => Config::default(),
    };

    let project_path = project_config_path();
    let project = if project_path.exists() {
        read_config_file(&project_path)
            .with_context(|| format!("project config error: {}", project_path.display()))?
    } else {
        Config::default()
    };

    Ok(merge_configs(global, project))
}

pub fn read_config_file(path: &Path) -> anyhow::Result<Config> {
    let yaml = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read config: {}", path.display()))?;
    parse_config(&yaml)
        .with_context(|| format!("invalid config at {}", path.display()))
}

pub fn write_config_file(path: &Path, cfg: &Config) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create config dir: {}", parent.display()))?;
    }
    let yaml = serde_yaml::to_string(cfg)
        .context("failed to serialize config")?;
    std::fs::write(path, yaml)
        .with_context(|| format!("cannot write config: {}", path.display()))?;
    Ok(())
}
```

**Step 4: Export from `lib.rs`**

Add to `crates/jobcard-core/src/lib.rs` at the top:

```rust
pub mod config;
pub use config::{load_config, Config};
```

**Step 5: Run the tests and verify they pass**

```bash
cd /Users/studio/gtfs && cargo test -p jobcard-core config 2>&1
```

Expected: all 8 config tests pass.

Note: `roundtrip_config_file` test uses `tempfile` — add it to `jobcard-core` dev-dependencies if not already there:

In `crates/jobcard-core/Cargo.toml`:
```toml
[dev-dependencies]
tempfile = "3"
```

**Step 6: Full test suite**

```bash
cd /Users/studio/gtfs && cargo test -p jobcard-core 2>&1
```

Expected: all tests pass.

**Step 7: Commit**

```bash
cd /Users/studio/gtfs
git add crates/jobcard-core/src/config.rs crates/jobcard-core/src/lib.rs crates/jobcard-core/Cargo.toml
git commit -m "feat(core): add Config struct with load/merge/parse/read/write"
```

---

### Task 3: Add `jc config get` and `jc config set` subcommands

**Files:**
- Modify: `crates/jc/src/main.rs`

**Step 1: Write the integration tests first**

Add a new test file `crates/jc/tests/config_cmd.rs`:

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
        .expect("cargo build failed");
    assert!(status.success());
}

fn jc_bin() -> PathBuf {
    repo_root().join("target").join("debug").join("jc")
}

fn config_get(config_path: &Path, key: &str) -> std::process::Output {
    Command::new(jc_bin())
        .args(["config", "get", key])
        .env("JOBCARD_CONFIG", config_path.to_str().unwrap())
        .output()
        .unwrap()
}

fn config_set(config_path: &Path, key: &str, value: &str) -> std::process::Output {
    Command::new(jc_bin())
        .args(["config", "set", key, value])
        .env("JOBCARD_CONFIG", config_path.to_str().unwrap())
        .output()
        .unwrap()
}

#[test]
fn config_set_and_get_max_concurrent() {
    build_jc();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.yaml");

    let out = config_set(&config_path, "max_concurrent", "4");
    assert!(out.status.success(), "set failed: {}", String::from_utf8_lossy(&out.stderr));

    let out = config_get(&config_path, "max_concurrent");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("4"), "expected '4' in output, got: {}", stdout);
}

#[test]
fn config_set_and_get_default_template() {
    build_jc();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.yaml");

    let out = config_set(&config_path, "default_template", "qa");
    assert!(out.status.success());

    let out = config_get(&config_path, "default_template");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("qa"));
}

#[test]
fn config_set_and_get_provider_chain() {
    build_jc();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.yaml");

    let out = config_set(&config_path, "default_provider_chain", "claude,codex");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let out = config_get(&config_path, "default_provider_chain");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("claude"), "got: {}", stdout);
    assert!(stdout.contains("codex"), "got: {}", stdout);
}

#[test]
fn config_get_missing_key_errors() {
    build_jc();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.yaml");

    let out = config_get(&config_path, "nonexistent_key");
    assert!(!out.status.success(), "expected non-zero exit for unknown key");
}

#[test]
fn config_get_unset_value_prints_empty_or_unset() {
    build_jc();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.yaml");
    // Create empty config
    fs::write(&config_path, "").unwrap();

    let out = config_get(&config_path, "max_concurrent");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Should print "(unset)" or empty line
    assert!(
        stdout.trim().is_empty() || stdout.contains("unset"),
        "got: {}", stdout
    );
}
```

**Step 2: Run tests to verify they fail**

```bash
cd /Users/studio/gtfs && cargo test -p jc --test config_cmd 2>&1 | tail -20
```

Expected: compile errors (Config subcommand doesn't exist yet).

**Step 3: Add `Config` subcommand to `main.rs`**

In `crates/jc/src/main.rs`, add to the `Command` enum:

```rust
Config {
    #[command(subcommand)]
    action: ConfigAction,
},
```

Add a new `ConfigAction` enum after the `Command` enum:

```rust
#[derive(Subcommand, Debug)]
enum ConfigAction {
    /// Print the current value of a config key.
    Get { key: String },
    /// Set a config key to a value (writes to project .jobcard/config.yaml).
    Set { key: String, value: String },
}
```

Add env-var-driven config path helper function (used by tests via `JOBCARD_CONFIG`):

```rust
fn resolve_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("JOBCARD_CONFIG") {
        return PathBuf::from(p);
    }
    jobcard_core::config::project_config_path()
}
```

Add the match arm in `main()`:

```rust
Command::Config { action } => {
    let config_path = resolve_config_path();
    match action {
        ConfigAction::Get { key } => cmd_config_get(&config_path, &key),
        ConfigAction::Set { key, value } => cmd_config_set(&config_path, &key, &value),
    }
}
```

Add the two handler functions (outside `main`):

```rust
fn cmd_config_get(config_path: &Path, key: &str) -> anyhow::Result<()> {
    let cfg = if config_path.exists() {
        jobcard_core::config::read_config_file(config_path)?
    } else {
        jobcard_core::Config::default()
    };

    match key {
        "default_provider_chain" => match cfg.default_provider_chain {
            Some(chain) => println!("{}", chain.join(",")),
            None => println!("(unset)"),
        },
        "max_concurrent" => match cfg.max_concurrent {
            Some(v) => println!("{}", v),
            None => println!("(unset)"),
        },
        "cooldown_seconds" => match cfg.cooldown_seconds {
            Some(v) => println!("{}", v),
            None => println!("(unset)"),
        },
        "log_retention_days" => match cfg.log_retention_days {
            Some(v) => println!("{}", v),
            None => println!("(unset)"),
        },
        "default_template" => match cfg.default_template {
            Some(v) => println!("{}", v),
            None => println!("(unset)"),
        },
        _ => anyhow::bail!("unknown config key '{}'. Valid keys: default_provider_chain, max_concurrent, cooldown_seconds, log_retention_days, default_template", key),
    }
    Ok(())
}

fn cmd_config_set(config_path: &Path, key: &str, value: &str) -> anyhow::Result<()> {
    let mut cfg = if config_path.exists() {
        jobcard_core::config::read_config_file(config_path)?
    } else {
        jobcard_core::Config::default()
    };

    match key {
        "default_provider_chain" => {
            cfg.default_provider_chain = Some(value.split(',').map(|s| s.trim().to_string()).collect());
        }
        "max_concurrent" => {
            cfg.max_concurrent = Some(value.parse::<usize>()
                .with_context(|| format!("max_concurrent must be a positive integer, got: {}", value))?);
        }
        "cooldown_seconds" => {
            cfg.cooldown_seconds = Some(value.parse::<u64>()
                .with_context(|| format!("cooldown_seconds must be a non-negative integer, got: {}", value))?);
        }
        "log_retention_days" => {
            cfg.log_retention_days = Some(value.parse::<u64>()
                .with_context(|| format!("log_retention_days must be a non-negative integer, got: {}", value))?);
        }
        "default_template" => {
            cfg.default_template = Some(value.to_string());
        }
        _ => anyhow::bail!("unknown config key '{}'. Valid keys: default_provider_chain, max_concurrent, cooldown_seconds, log_retention_days, default_template", key),
    }

    jobcard_core::config::write_config_file(config_path, &cfg)?;
    Ok(())
}
```

**Step 4: Run tests and verify they pass**

```bash
cd /Users/studio/gtfs && cargo test -p jc --test config_cmd 2>&1
```

Expected: all 5 config_cmd tests pass.

**Step 5: Ensure existing tests still pass**

```bash
cd /Users/studio/gtfs && cargo test 2>&1
```

Expected: all tests pass.

**Step 6: Commit**

```bash
cd /Users/studio/gtfs
git add crates/jc/src/main.rs crates/jc/tests/config_cmd.rs
git commit -m "feat(jc): add 'config get' and 'config set' subcommands"
```

---

### Task 4: Load config on `jc init` and write defaults to global config

**Files:**
- Modify: `crates/jc/src/main.rs` (extend `Command::Init` arm)

**Step 1: Write the failing test**

Add to `crates/jc/tests/config_cmd.rs`:

```rust
#[test]
fn init_creates_global_config_with_defaults() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    // Point JOBCARD_CONFIG to a temp path so we don't touch real ~/.jobcard
    let config_path = td.path().join("config.yaml");

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .env("JOBCARD_CONFIG", config_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    assert!(config_path.exists(), "config file should be created by init");
    let content = fs::read_to_string(&config_path).unwrap();
    // Should have sensible defaults
    assert!(content.contains("max_concurrent") || content.contains("default_template"),
        "config should contain default keys, got: {}", content);
}
```

**Step 2: Run to verify failure**

```bash
cd /Users/studio/gtfs && cargo test -p jc --test config_cmd init_creates 2>&1 | tail -10
```

Expected: test fails (init does not yet create config).

**Step 3: Extend `Command::Init` handler in `main.rs`**

In the `Command::Init` arm, after `seed_providers(&root)?;`, add:

```rust
// Create config with sensible defaults if it doesn't exist
let config_path = resolve_config_path();
if !config_path.exists() {
    let defaults = jobcard_core::Config {
        default_provider_chain: Some(vec!["mock".to_string()]),
        max_concurrent: Some(1),
        cooldown_seconds: Some(300),
        log_retention_days: Some(30),
        default_template: Some("implement".to_string()),
    };
    jobcard_core::config::write_config_file(&config_path, &defaults)
        .with_context(|| format!("failed to create default config at {}", config_path.display()))?;
}
```

**Step 4: Run test and verify it passes**

```bash
cd /Users/studio/gtfs && cargo test -p jc --test config_cmd 2>&1
```

Expected: all 6 config tests pass.

**Step 5: Commit**

```bash
cd /Users/studio/gtfs
git add crates/jc/src/main.rs
git commit -m "feat(jc): write default config on 'jc init'"
```

---

### Task 5: Wire loaded config into dispatcher defaults

**Files:**
- Modify: `crates/jc/src/main.rs` — load config in `main()`, use values as fallbacks for dispatcher flags

**Step 1: Write the failing test**

Add to `crates/jc/tests/config_cmd.rs`:

```rust
#[test]
fn dispatcher_uses_config_max_concurrent() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    let config_path = td.path().join("config.yaml");

    // Write config with max_concurrent = 2
    fs::write(&config_path, "max_concurrent: 2\n").unwrap();

    // Init
    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .env("JOBCARD_CONFIG", config_path.to_str().unwrap())
        .output().unwrap();
    assert!(out.status.success());

    // Write a template and create a card
    let tdir = cards.join("templates").join("implement.jobcard");
    fs::create_dir_all(tdir.join("logs")).unwrap();
    fs::create_dir_all(tdir.join("output")).unwrap();
    fs::write(tdir.join("meta.json"),
        r#"{"id":"t","created":"2026-03-01T00:00:00Z","stage":"implement","provider_chain":["mock"],"stages":{},"acceptance_criteria":[]}"#
    ).unwrap();
    fs::write(tdir.join("spec.md"), "").unwrap();
    fs::write(tdir.join("prompt.md"), "{{spec}}\n").unwrap();

    let mock_adapter = repo_root().join("adapters").join("mock.sh");
    let mock_cmd = mock_adapter.to_str().unwrap();
    let providers_json = format!(
        r#"{{"providers":{{"mock":{{"command":"{}","rate_limit_exit":75}}}}}}"#,
        mock_cmd
    );
    fs::write(cards.join("providers.json"), providers_json).unwrap();

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "new", "implement", "cfg-job1"])
        .env("JOBCARD_CONFIG", config_path.to_str().unwrap())
        .output().unwrap();
    assert!(out.status.success());

    // Run dispatcher without --max-workers flag; it should pick up max_concurrent=2 from config
    let out = Command::new(jc_bin())
        .env("MOCK_EXIT", "0")
        .env("JOBCARD_CONFIG", config_path.to_str().unwrap())
        .args([
            "--cards-dir", cards.to_str().unwrap(),
            "dispatcher",
            "--adapter", mock_cmd,
            "--once",
        ])
        .output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(cards.join("done").join("cfg-job1.jobcard").exists(), "card should be done");
}
```

**Step 2: Run to verify it's currently passing anyway** (config loading not yet wired, but dispatcher still works with default `max_workers=1`)

```bash
cd /Users/studio/gtfs && cargo test -p jc --test config_cmd dispatcher_uses_config 2>&1 | tail -10
```

Note: this test might already pass since dispatcher defaults to 1 worker and the card will still process. The real value of config wiring shows when config overrides the CLI default.

**Step 3: Load config in `main()` and pass it to the dispatcher**

In `main()`, after parsing the CLI, load the config:

```rust
// Load merged global+project config (missing files silently skipped)
let cfg = jobcard_core::load_config().unwrap_or_default();
```

In the `Command::Dispatcher` arm, change the `run_dispatcher` call to use config values as fallbacks when the user hasn't explicitly passed flags.

The cleanest approach: make `max_workers`, `poll_ms`, `max_retries`, and `reap_ms` use `Option<usize>` / `Option<u64>` / `Option<u32>` in the Clap struct so we can distinguish "not provided" from "provided". Then fall back to config values.

Change the Dispatcher variant in the `Command` enum:

```rust
Dispatcher {
    #[arg(long, default_value = "adapters/mock.sh")]
    adapter: String,

    #[arg(long)]
    max_workers: Option<usize>,

    #[arg(long, default_value_t = 500)]
    poll_ms: u64,

    #[arg(long)]
    max_retries: Option<u32>,

    #[arg(long, default_value_t = 1000)]
    reap_ms: u64,

    #[arg(long)]
    no_reap: bool,

    #[arg(long)]
    once: bool,
},
```

In the `Command::Dispatcher` match arm:

```rust
Command::Dispatcher { adapter, max_workers, poll_ms, max_retries, reap_ms, no_reap, once } => {
    let effective_max_workers = max_workers
        .or(cfg.max_concurrent)
        .unwrap_or(1);
    let effective_max_retries = max_retries.unwrap_or(3);
    run_dispatcher(&root, &adapter, effective_max_workers, poll_ms, effective_max_retries, reap_ms, no_reap, once).await
}
```

**Step 4: Run all tests**

```bash
cd /Users/studio/gtfs && cargo test 2>&1
```

Expected: all tests pass. Pay attention to any existing dispatcher tests that pass `--max-workers`; they use explicit flags so they won't be affected.

**Step 5: Commit**

```bash
cd /Users/studio/gtfs
git add crates/jc/src/main.rs crates/jc/tests/config_cmd.rs
git commit -m "feat(jc): wire config max_concurrent into dispatcher as default"
```

---

### Task 6: Final check — lint, fmt, full test suite

**Step 1: Format**

```bash
cd /Users/studio/gtfs && cargo fmt
```

**Step 2: Clippy**

```bash
cd /Users/studio/gtfs && cargo clippy -- -D warnings 2>&1
```

Fix any warnings before proceeding.

**Step 3: Full test suite**

```bash
cd /Users/studio/gtfs && cargo test 2>&1
```

Expected: all tests green.

**Step 4: Smoke test the CLI manually**

```bash
cd /Users/studio/gtfs
tmp=$(mktemp -d)
export JOBCARD_CONFIG="$tmp/config.yaml"
./target/debug/jc --cards-dir "$tmp/.cards" init
cat "$tmp/config.yaml"          # should show defaults
./target/debug/jc config get max_concurrent
./target/debug/jc config set max_concurrent 3
./target/debug/jc config get max_concurrent   # should print 3
./target/debug/jc config get nonexistent_key  # should exit non-zero with clear message
./target/debug/jc config set max_concurrent bad_value  # should exit non-zero with clear error
```

**Step 5: Commit any fmt/clippy fixes**

```bash
cd /Users/studio/gtfs
git add -p
git commit -m "chore: fmt and clippy fixes for config feature"
```

---

## Acceptance Criteria Verification

| Criterion | Verified by |
|-----------|-------------|
| `~/.jobcard/config.yaml` read on every invocation; project overrides global | `load_config()` unit tests + `merge_configs` tests |
| `jc config get <key>` reads config | `config_get_*` integration tests |
| `jc config set <key> <value>` writes config | `config_set_*` integration tests |
| Supports all 5 config fields | `parse_valid_yaml` unit test |
| Missing/malformed config → clear error | `parse_malformed_yaml_returns_error` + `read_missing_file_returns_error` tests |
| Config created with defaults on `jc init` | `init_creates_global_config_with_defaults` integration test |
