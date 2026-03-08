# bop list --json Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `--json` flag to `bop list` that emits newline-delimited JSON (ndjson), one `Meta` object per line with `state` and `team` fields added, making bop a Unix citizen pipeable to jq/nu/fzf/rg.

**Architecture:** Single flag threads from the CLI arg definition in `main.rs` through to `list.rs`. A thin `JsonCard` wrapper struct adds `state` + `team` to each serialized `Meta`. ANSI path is unchanged. JSON path collects all cards, emits one JSON line per card to stdout.

**Tech Stack:** Rust, clap (existing), serde_json (existing workspace dep), bop_core::Meta (existing).

---

### Task 1: Add `--json` flag and ndjson output to `bop list`

**Files:**
- Modify: `crates/bop-cli/src/main.rs` (List variant, ~line 128)
- Modify: `crates/bop-cli/src/list.rs` (list_cards signature + JSON path)

---

**Step 1: Write failing tests**

Add to the `tests` module in `crates/bop-cli/src/list.rs`:

```rust
#[test]
fn list_cards_json_emits_ndjson_lines() {
    let td = tempdir().unwrap();
    setup_card_in_state(td.path(), "pending", "task-alpha");
    setup_card_in_state(td.path(), "running", "task-beta");

    // Capture stdout
    let mut out = Vec::<u8>::new();
    list_cards_json(td.path(), "all", &mut out).unwrap();

    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "one JSON line per card");

    // Each line must parse as valid JSON with state + id fields
    for line in &lines {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("invalid JSON line: {e}: {line}"));
        assert!(v.get("id").is_some(), "must have id field");
        assert!(v.get("state").is_some(), "must have state field");
    }
}

#[test]
fn list_cards_json_state_field_matches_directory() {
    let td = tempdir().unwrap();
    setup_card_in_state(td.path(), "failed", "my-card");

    let mut out = Vec::<u8>::new();
    list_cards_json(td.path(), "failed", &mut out).unwrap();

    let text = String::from_utf8(out).unwrap();
    let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(v["state"], "failed");
    assert_eq!(v["id"], "my-card");
}

#[test]
fn list_cards_json_team_field_present_for_team_dirs() {
    let td = tempdir().unwrap();
    let team_root = td.path().join("team-cli");
    fs::create_dir_all(&team_root).unwrap();
    setup_card_in_state(&team_root, "pending", "cli-task");

    let mut out = Vec::<u8>::new();
    list_cards_json(td.path(), "pending", &mut out).unwrap();

    let text = String::from_utf8(out).unwrap();
    let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(v["team"], "team-cli");
}

#[test]
fn list_cards_json_empty_produces_no_output() {
    let td = tempdir().unwrap();
    fs::create_dir_all(td.path().join("pending")).unwrap();

    let mut out = Vec::<u8>::new();
    list_cards_json(td.path(), "all", &mut out).unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(text.is_empty(), "no cards = no output");
}
```

**Step 2: Run tests to verify they fail**

```sh
cargo test -p bop list::tests::list_cards_json 2>&1 | head -20
```

Expected: FAIL — `list_cards_json` does not exist yet.

---

**Step 3: Add `list_cards_json` to `list.rs`**

Add this near the top of `list.rs` after the existing `use` statements:

```rust
use std::io::Write;
```

Add this struct and function after `list_cards`:

```rust
#[derive(serde::Serialize)]
struct JsonCard<'a> {
    state: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    team: Option<String>,
    #[serde(flatten)]
    meta: &'a bop_core::Meta,
}

pub fn list_cards_json(root: &Path, state_filter: &str, out: &mut impl Write) -> anyhow::Result<()> {
    let states: Vec<&str> = match state_filter {
        "all" => vec!["drafts", "pending", "running", "done", "failed", "merged"],
        "active" => vec!["pending", "running", "done"],
        "drafts" => vec!["drafts"],
        other => vec![other],
    };

    for state in &states {
        emit_state_json(root, state, None, out)?;

        if let Ok(entries) = fs::read_dir(root) {
            let mut team_dirs: Vec<_> = entries
                .flatten()
                .filter(|e| {
                    let name = e.file_name();
                    let s = name.to_string_lossy();
                    e.path().is_dir() && s.starts_with("team-")
                })
                .collect();
            team_dirs.sort_by_key(|e| e.file_name());
            for entry in team_dirs {
                emit_state_json(
                    &entry.path(),
                    state,
                    Some(entry.file_name().to_string_lossy().into_owned()),
                    out,
                )?;
            }
        }
    }
    Ok(())
}

fn emit_state_json(
    dir: &Path,
    state: &str,
    team: Option<String>,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let state_dir = dir.join(state);
    if !state_dir.exists() {
        return Ok(());
    }

    if let Ok(entries) = fs::read_dir(&state_dir) {
        let mut metas: Vec<bop_core::Meta> = entries
            .flatten()
            .filter(|e| e.path().is_dir() && e.path().extension().is_some_and(|x| x == "bop"))
            .filter_map(|e| bop_core::read_meta(&e.path()).ok())
            .collect();
        metas.sort_by(|a, b| a.id.cmp(&b.id));

        for meta in &metas {
            let card = JsonCard {
                state,
                team: team.clone(),
                meta,
            };
            writeln!(out, "{}", serde_json::to_string(&card)?)?;
        }
    }
    Ok(())
}
```

**Step 4: Run tests to verify they pass**

```sh
cargo test -p bop list::tests::list_cards_json 2>&1
```

Expected: 4 tests pass.

---

**Step 5: Wire `--json` flag into the CLI**

In `crates/bop-cli/src/main.rs`, update the `List` variant (around line 128):

```rust
    /// List cards with glyphs, stages, and progress.
    List {
        /// Filter: pending, running, done, failed, merged, active (default), all.
        #[arg(long, default_value = "active")]
        state: String,
        /// Emit newline-delimited JSON (ndjson) instead of ANSI table.
        /// One JSON object per line: Meta fields + "state" + "team".
        /// Pipe to: jq, nu, fzf, rg for filtering and selection.
        #[arg(long)]
        json: bool,
    },
```

Update the dispatch arm (around line 450):

```rust
        Command::List { state, json } => {
            if json {
                list::list_cards_json(&root, &state, &mut std::io::stdout())
            } else {
                list::list_cards(&root, &state)
            }
        }
```

**Step 6: Run full check**

```sh
make check
```

Expected: all tests pass, clippy clean, fmt clean.

---

**Step 7: Smoke test the flag**

```sh
# Create a test card if none exist
mkdir -p /tmp/bop-test/pending/smoke-test.bop
echo '{"id":"smoke-test","created":"2026-03-04T00:00:00Z","stage":"spec"}' \
  > /tmp/bop-test/pending/smoke-test.bop/meta.json

# Run the binary against it
./target/debug/bop --root /tmp/bop-test list --json
```

Expected output (one line):
```json
{"id":"smoke-test","created":"2026-03-04T00:00:00Z","state":"pending","stage":"spec"}
```

Then confirm piping works:
```sh
./target/debug/bop list --json --state all | jq 'select(.stage == "spec")'
```

---

**Step 8: Commit**

```sh
jj new -m "feat: add --json flag to bop list (ndjson output, Unix-pipeable)"
# stage relevant files
jj commit
```

---

## Usage examples (for docs/comments)

```sh
# All running cards
bop list --json --state running | jq .id

# Cards in qa stage across all states
bop list --json --state all | jq 'select(.stage == "qa")'

# Interactive card picker with fzf
bop list --json --state all | fzf --preview 'bop inspect {.id}' | jq -r .id

# Nu pipeline: blocked cards with progress < 50
bop list --json --state all | lines | each { from json } | where { $in.progress? | default 0 | $in < 50 }

# ripgrep: find cards mentioning a keyword in their id
bop list --json | rg "feat-auth"
```
