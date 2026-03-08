# Spec 034 — bop bridge: session state bridge for BopDeck

## Goal

Add a `bop bridge` subcommand that maps live session events from all AI CLI
tools (Claude Code, opencode, Goose, Gemini, Codex, Aider, Ollama) to bop
card state transitions, surfacing them in BopDeck's notch and status display.

Cards move through five stages as work progresses:

```
planning → in-progress → human-review → ai-review → done
```

The bridge emits `BridgeEvent` JSON to a Unix domain socket at
`~/.bop/bridge.sock`. BopDeck reads that socket and animates the card in the
notch. No database, no polling loop for the daemon — it reacts to events.

## Background

Different CLIs expose session state in incompatible ways:

| CLI | Event mechanism |
|-----|----------------|
| Claude Code | `~/.claude/settings.json` hook system (17+ events) |
| opencode | SSE at `localhost:4096/event` |
| Goose | SSE at `localhost:PORT/reply`, `POST /action-required/tool-confirmation` |
| Gemini CLI | ACP ndjson stream (`@agentclientprotocol/sdk`) |
| Aider | Log file tail (`~/.aider.log` or `.aider.chat.history.md`) |
| Crush | Log file tail |
| Ollama | REST poll `GET /api/ps` |

Rather than baking a different integration per CLI, the bridge exposes a
universal **emit** interface (`bop bridge emit <event>`) that any hook,
adapter, or skill can call. A lightweight **listen** mode reads the socket and
prints events for debugging.

## Deliverables

### 1. `crates/bop-cli/src/bridge/mod.rs`

Socket path, event types, serialization:

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Unix socket path: `~/.bop/bridge.sock`
pub fn socket_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".bop").join("bridge.sock"))
}

/// Five canonical card stages (BopDeck state machine).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CardStage {
    Planning,
    InProgress,
    HumanReview,
    AiReview,
    Done,
}

/// Session events emitted by CLI adapters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "event")]
pub enum BridgeEvent {
    /// A CLI session started.
    SessionStart {
        cli: String,          // "claude", "opencode", "goose", "aider", "gemini", "ollama"
        session_id: String,
        card_id: Option<String>,
    },
    /// Card stage changed.
    StageChange {
        cli: String,
        session_id: String,
        card_id: Option<String>,
        stage: CardStage,
    },
    /// A tool call started (PreToolUse equivalent).
    ToolStart {
        cli: String,
        session_id: String,
        tool: String,
    },
    /// A tool call finished.
    ToolDone {
        cli: String,
        session_id: String,
        tool: String,
        success: bool,
    },
    /// AI is waiting for human input / permission.
    AwaitingHuman {
        cli: String,
        session_id: String,
        reason: Option<String>,
    },
    /// Session ended.
    SessionEnd {
        cli: String,
        session_id: String,
        card_id: Option<String>,
        exit_ok: bool,
    },
}

/// Write one JSON-line event to the Unix socket.
/// Non-fatal if socket not present — bridge may not be running.
pub fn emit(event: &BridgeEvent) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let path = match socket_path() {
        Some(p) => p,
        None => return Ok(()),
    };
    if !path.exists() {
        return Ok(()); // bridge not running — silently skip
    }
    let mut stream = UnixStream::connect(&path)?;
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    Ok(())
}
```

### 2. `crates/bop-cli/src/bridge/hook_installer.rs`

Installs Claude Code hooks into `~/.claude/settings.json`:

```rust
/// Install Claude Code hooks that emit BridgeEvents.
///
/// Writes (or merges) into `~/.claude/settings.json`:
/// - SessionStart → `bop bridge emit session-start --cli claude --session $SESSION_ID`
/// - PreToolUse   → `bop bridge emit tool-start --cli claude --session $SESSION_ID --tool $TOOL_NAME`
/// - PostToolUse  → `bop bridge emit tool-done --cli claude --session $SESSION_ID --tool $TOOL_NAME`
/// - Stop         → `bop bridge emit session-end --cli claude --session $SESSION_ID`
pub fn install_claude_hooks(bop_bin: &Path) -> anyhow::Result<()> { ... }

/// Remove bop hooks from `~/.claude/settings.json`.
pub fn uninstall_claude_hooks() -> anyhow::Result<()> { ... }
```

`~/.claude/settings.json` schema (merged, not overwritten):

```json
{
  "hooks": {
    "SessionStart": [{
      "type": "command",
      "command": "/path/to/bop bridge emit --cli claude --event session-start"
    }],
    "PreToolUse": [{
      "type": "command",
      "command": "/path/to/bop bridge emit --cli claude --event tool-start --tool $CLAUDE_TOOL_NAME"
    }],
    "PostToolUse": [{
      "type": "command",
      "command": "/path/to/bop bridge emit --cli claude --event tool-done --tool $CLAUDE_TOOL_NAME"
    }],
    "Stop": [{
      "type": "command",
      "command": "/path/to/bop bridge emit --cli claude --event session-end"
    }]
  }
}
```

Read existing settings JSON → merge `hooks` key → write back (pretty-printed,
do not clobber other settings). If `hooks` already has a `SessionStart` array,
append rather than replace.

### 3. `crates/bop-cli/src/bridge/opencode.rs`

SSE listener for opencode's event bus:

```rust
/// Connect to opencode's SSE bus and translate events to BridgeEvents.
///
/// GET http://localhost:{port}/event
/// Events: session.status (idle/busy/retry) → StageChange or AwaitingHuman
///         message.part.delta → ToolStart
///         permission.updated → AwaitingHuman
pub async fn listen_opencode(port: u16) -> anyhow::Result<()> { ... }
```

Port discovery: check `$OPENCODE_PORT` env var, else default 4096.

### 4. `crates/bop-cli/src/bridge/cmd.rs`

CLI subcommand handler:

```rust
pub enum BridgeSubcommand {
    /// Start the Unix socket listener (daemon mode).
    Listen {
        #[arg(long, default_value = "false")]
        verbose: bool,
    },
    /// Emit a single event to the socket (called by hooks/scripts).
    Emit {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        event: String,          // "session-start", "tool-start", "tool-done", "session-end", "awaiting-human", "stage-change"
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        card_id: Option<String>,
        #[arg(long)]
        tool: Option<String>,
        #[arg(long)]
        stage: Option<String>,  // "planning", "in-progress", "human-review", "ai-review", "done"
    },
    /// Install Claude Code hooks.
    Install {
        #[arg(long, value_enum, default_value = "claude")]
        target: HookTarget,
    },
    /// Remove installed hooks.
    Uninstall {
        #[arg(long, value_enum, default_value = "claude")]
        target: HookTarget,
    },
    /// Connect to opencode SSE bus and forward events.
    Opencode {
        #[arg(long, default_value = "4096")]
        port: u16,
    },
}

#[derive(Clone, ValueEnum)]
pub enum HookTarget {
    Claude,
}

pub async fn cmd_bridge(sub: BridgeSubcommand) -> anyhow::Result<()> { ... }
```

#### `bridge listen` — Unix socket daemon

```
~/.bop/bridge.sock  (SOCK_STREAM, accept loop)
```

- Creates socket, accepts connections in a loop
- Each connection: read newline-delimited JSON, deserialize `BridgeEvent`, print to stdout if `--verbose`
- Writes events to `~/.bop/bridge-events.jsonl` (append, same O_APPEND atomic pattern as provider history)
- On SIGTERM/Ctrl-C: remove socket file and exit cleanly

Socket is non-exclusive — if `~/.bop/bridge.sock` already exists and is
connectable, `bridge listen` exits with a helpful message ("bridge already
running"). If stale (connect fails), removes and recreates.

#### `bridge emit` — fire-and-forget hook caller

```
bop bridge emit --cli claude --event session-start --session "$CLAUDE_SESSION_ID"
```

- Constructs a `BridgeEvent` from flags
- Calls `emit(&event)` — connects to socket, writes JSON line, closes
- If socket not present: silently succeeds (bridge not running is OK)
- Exit 0 always — hooks must not fail sessions

### 5. `vibekanban/bop-bridge.nu` — Nushell skill script

A Nushell script that can be sourced by `.card/` prompts to give any AI
session the `bop-bridge` skill. The script wraps `bop bridge emit` calls in
natural language instructions:

```nushell
#!/usr/bin/env nu
# bop-bridge.nu — emit card stage transitions from inside any AI session

# Usage: source this file, then call stage-change with the desired stage.
# The AI is instructed to call these when appropriate.

def "bridge stage" [stage: string, card_id?: string] {
    mut args = ["bridge", "emit", "--cli", "claude", "--event", "stage-change", "--stage", $stage]
    if $card_id != null {
        $args = ($args | append ["--card-id", $card_id])
    }
    ^bop ...$args
}
```

The bop `prompt.md` template gains a `{{bridge_skill}}` substitution:

```markdown
## Session bridge

You have the `bop-bridge` skill. Call these commands at natural transition points:

- When you begin active work: `bop bridge emit --cli claude --event stage-change --stage in-progress`
- When you need human review: `bop bridge emit --cli claude --event stage-change --stage human-review`
- When you trigger QA review: `bop bridge emit --cli claude --event stage-change --stage ai-review`
- When fully done: `bop bridge emit --cli claude --event stage-change --stage done`

These are non-blocking — run them with `&&` after the triggering action.
```

### 6. `crates/bop-cli/src/main.rs` — wire bridge subcommand

Add `Bridge` variant to `Command` enum:

```rust
/// Session state bridge — connects AI CLI events to BopDeck.
Bridge {
    #[command(subcommand)]
    sub: BridgeSubcommand,
},
```

Route:

```rust
Command::Bridge { sub } => bridge::cmd_bridge(sub).await,
```

### 7. Unit tests

`crates/bop-cli/src/bridge/mod.rs` `#[cfg(test)]` block:

- `test_event_serialize_session_start`: serialize `SessionStart` → verify JSON `"event": "session-start"` and snake/kebab-case
- `test_event_serialize_stage_change`: serialize `StageChange { stage: CardStage::InProgress }` → verify `"stage": "in-progress"`
- `test_emit_no_socket_is_noop`: call `emit()` when socket doesn't exist — returns `Ok(())`
- `test_roundtrip_all_variants`: serialize then deserialize all `BridgeEvent` variants
- `test_stage_from_str`: `"human-review"` parses to `CardStage::HumanReview`

`crates/bop-cli/src/bridge/hook_installer.rs` `#[cfg(test)]` block:

- `test_install_into_empty_settings`: writes correct hooks when `settings.json` doesn't exist
- `test_install_merges_existing_settings`: existing non-hook settings preserved after install
- `test_install_appends_existing_hooks`: existing hooks for same event get bop hook appended, not replaced
- `test_uninstall_removes_bop_hooks_only`: other hooks remain intact after uninstall

## Acceptance criteria

```sh
# 1. Build succeeds
cargo build -p bop 2>&1 | grep -v warning | grep -c error | grep -q '^0$'

# 2. All tests pass
cargo test -p bop bridge:: 2>&1 | tail -5 | grep -q 'test result: ok'

# 3. make check passes
make check

# 4. bridge emit subcommand exists and exits 0 (no socket running = noop)
./target/debug/bop bridge emit --cli claude --event session-start --session test-123

# 5. bridge install wires Claude Code hooks into settings.json
./target/debug/bop bridge install --target claude && \
  grep -q '"SessionStart"' ~/.claude/settings.json && \
  grep -q 'bop bridge emit' ~/.claude/settings.json

# 6. bridge listen creates socket then exits on interrupt
timeout 2 ./target/debug/bop bridge listen || true
# socket removed after exit:
! test -S ~/.bop/bridge.sock

# 7. listen + emit roundtrip
./target/debug/bop bridge listen --verbose &
LISTEN_PID=$!
sleep 0.3
./target/debug/bop bridge emit --cli claude --event stage-change --stage in-progress
sleep 0.3
kill $LISTEN_PID
wait $LISTEN_PID 2>/dev/null
grep -q 'stage-change' ~/.bop/bridge-events.jsonl
```

## Dependencies

No new crates required. Uses existing:
- `tokio` (with `net` feature — already added to Cargo.toml)
- `serde` / `serde_json`
- `dirs`
- `anyhow`
- `clap`
- Unix socket: `std::os::unix::net::UnixListener` / `UnixStream`

## Files to create

- `crates/bop-cli/src/bridge/mod.rs`
- `crates/bop-cli/src/bridge/cmd.rs`
- `crates/bop-cli/src/bridge/hook_installer.rs`
- `crates/bop-cli/src/bridge/opencode.rs`
- `vibekanban/bop-bridge.nu`

## Files to modify

- `crates/bop-cli/src/main.rs` — add `Bridge` variant + route
- `crates/bop-cli/Cargo.toml` — no new deps needed (tokio net already added)

## Out of scope (next spec: 035)

- Goose SSE adapter
- Gemini ACP ndjson consumer
- Aider / Crush log tailer
- Ollama REST poller
- BopDeck-side socket reader (that's in the BopDeck notch spec)
- `{{bridge_skill}}` template substitution wiring in `render_prompt`
