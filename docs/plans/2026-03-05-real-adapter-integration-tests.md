# Real Adapter Integration Tests Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Prove that `claude.nu`, `ollama-local.nu`, and `codex.nu` each produce real LLM output when wired through the dispatcher — a card enters `pending/`, the real tool runs, `output/result.md` is written, and the card lands in `done/`.

**Architecture:** A single Nushell script `scripts/test-real-adapters.nu` creates a temp `.cards/` directory, writes a `providers.json` wiring each adapter, drops a trivial card, runs the dispatcher, and asserts `done/` + `output/result.md`. Each adapter section skips gracefully when the tool is absent. Ollama uses the already-installed `qwen2.5:7b` model. Claude and Codex use their real CLIs (both installed). Tests are isolated from `.cards/` — they use `mktemp -d`.

**Tech Stack:** Nushell 0.111, `bop` binary (`./target/debug/bop`), `claude` CLI, `codex` CLI, `ollama` (localhost:11434, qwen2.5:7b).

---

### Providers.json format reference

The dispatcher reads `.cards/providers.json` as a `ProvidersFile` struct:

```json
{
  "default_provider": "claude",
  "providers": {
    "claude": {
      "command": "/path/to/adapters/claude.nu",
      "rate_limit_exit": 75
    }
  }
}
```

Card `meta.json` must include `"provider_chain": ["claude"]` — the dispatcher's `select_provider` reads this field.

---

### Task 1: Self-test all three adapter `--test` flags pass

**Files:**
- No file changes needed — existing `run_tests` in each adapter

**Step 1: Run existing self-tests**

```sh
nu adapters/claude.nu --test
nu adapters/ollama-local.nu --test
nu adapters/codex.nu --test
```

Expected: `PASS: claude.nu`, `PASS: ollama-local.nu`, `PASS: codex.nu`

These only exercise logic (path resolution, exit-code mapping, rate-limit detection) — NOT real LLM calls. This task establishes baseline before the real tests.

**Step 2: Commit baseline (if any fixes needed)**

```sh
jj describe -m "test: verify adapter self-tests pass"
```

---

### Task 2: Create `scripts/test-real-adapters.nu`

**Files:**
- Create: `scripts/test-real-adapters.nu`

This script tests each adapter by running the full dispatcher pipeline against a real tool invocation.

**The trivial prompt** — each card gets this as `spec.md`:

```
Write the exact text "hello from <adapter>" (where <adapter> is the adapter name) to a file
called output/result.md and nothing else. Do not explain. Do not write any other files.
Create the output/ directory if it doesn't exist.
```

**Step 1: Write the script**

```nushell
#!/usr/bin/env nu
# test-real-adapters.nu — end-to-end integration test for claude, ollama, codex adapters
#
# Usage:
#   nu scripts/test-real-adapters.nu           # test all available adapters
#   nu scripts/test-real-adapters.nu --adapter claude   # test one adapter

def main [
    --adapter: string = "all"   # which adapter to test: claude | ollama | codex | all
    --timeout: int = 120        # seconds to wait for card to reach done/
] {
    let root = ($env.CURRENT_FILE | path dirname | path dirname)
    let bop = $"($root)/target/debug/bop"
    let adapters_dir = $"($root)/adapters"

    if not ($bop | path exists) {
        print -e $"error: bop binary not found at ($bop) — run: cargo build"
        exit 1
    }

    let results = []

    let to_test = if $adapter == "all" {
        ["claude", "ollama", "codex"]
    } else {
        [$adapter]
    }

    mut passed = 0
    mut failed = 0
    mut skipped = 0

    for name in $to_test {
        let result = (run_adapter_test $name $bop $adapters_dir $timeout)
        match $result {
            "pass"    => { $passed += 1;  print $"✓ ($name)" }
            "skip"    => { $skipped += 1; print $"⊘ ($name) — skipped (tool not available)" }
            "fail"    => { $failed += 1;  print $"✗ ($name) — FAILED" }
        }
    }

    print $"\n($passed) passed  ($skipped) skipped  ($failed) failed"

    if $failed > 0 { exit 1 }
}

def run_adapter_test [name: string, bop: string, adapters_dir: string, timeout_s: int]: nothing -> string {
    # Check prerequisite
    let available = match $name {
        "claude" => ((which claude | length) > 0)
        "ollama" => (
            (which ollama | length) > 0
            and ((do { ^curl -sf "http://localhost:11434/api/tags" } | complete | get exit_code) == 0)
        )
        "codex"  => ((which codex | length) > 0)
        _        => false
    }

    if not $available {
        return "skip"
    }

    # Create isolated temp cards dir
    let td = (^mktemp -d /tmp/bop-adapter-test-XXXXXX | str trim)

    # Create directory structure
    mkdir $"($td)/pending"
    mkdir $"($td)/running"
    mkdir $"($td)/done"
    mkdir $"($td)/failed"

    # Write providers.json
    let adapter_path = $"($adapters_dir)/($name).nu"
    let providers = {
        default_provider: $name,
        providers: {
            $name: {
                command: $adapter_path,
                rate_limit_exit: 75
            }
        }
    }
    $providers | to json | save --force $"($td)/providers.json"

    # Write card
    let card_id = $"test-($name)-adapter"
    let card_dir = $"($td)/pending/($card_id).bop"
    mkdir $card_dir
    mkdir $"($card_dir)/output"

    let meta = {
        id: $card_id,
        created: (date now | format date "%Y-%m-%dT%H:%M:%SZ"),
        stage: "implement",
        provider_chain: [$name]
    }
    $meta | to json | save --force $"($card_dir)/meta.json"

    let prompt = $"Write the exact text \"hello from ($name)\" to a file called output/result.md. Do not write anything else. Create the output/ directory if needed."
    $prompt | save --force $"($card_dir)/spec.md"
    $prompt | save --force $"($card_dir)/prompt.md"

    # Run dispatcher in background — poll aggressively, stop after card is done
    let log = $"($td)/dispatcher.log"
    let start = (date now)

    # Run dispatcher with short poll (watcher + 2s fallback), --once mode
    let result = (do {
        ^($bop) --cards-dir $td dispatcher --poll-ms 2000 --adapter $adapter_path --no-reap
    } | complete)

    # Wait for card to appear in done/ (poll up to timeout_s)
    mut elapsed = 0
    mut card_done = false
    while $elapsed < $timeout_s {
        if ($"($td)/done/($card_id).bop" | path exists) {
            $card_done = true
            break
        }
        ^sleep 2
        $elapsed += 2
    }

    if not $card_done {
        print -e $"  ($name): card did not reach done/ within ($timeout_s)s"
        print -e $"  pending: (ls $"($td)/pending/" | length)  failed: (ls $"($td)/failed/" | length)"
        ^rm -rf $td
        return "fail"
    }

    # Check output/result.md exists
    let result_md = $"($td)/done/($card_id).bop/output/result.md"
    if not ($result_md | path exists) {
        print -e $"  ($name): output/result.md missing from done card"
        ^rm -rf $td
        return "fail"
    }

    let content = (open --raw $result_md | str trim)
    if ($content | str length) == 0 {
        print -e $"  ($name): output/result.md is empty"
        ^rm -rf $td
        return "fail"
    }

    print $"  ($name): output/result.md = '($content | str substring 0..80)'"
    ^rm -rf $td
    return "pass"
}
```

**Step 2: Make executable and run self-check (no real tools yet)**

```sh
chmod +x scripts/test-real-adapters.nu
# Dry-run: verify the script parses cleanly
nu --no-config-file scripts/test-real-adapters.nu --help 2>&1 || true
```

Expected: usage printed, no parse errors.

**Step 3: Commit**

```sh
jj describe -m "feat: add scripts/test-real-adapters.nu — end-to-end adapter integration tests"
```

---

### Task 3: Run ollama end-to-end test

Ollama is available locally with `qwen2.5:7b` — the cheapest real test (no API key, no cost).

**Step 1: Run the ollama test**

```sh
OLLAMA_MODEL=qwen2.5:7b nu --no-config-file scripts/test-real-adapters.nu --adapter ollama --timeout 180
```

Expected:
```
✓ ollama
  ollama: output/result.md = 'hello from ollama'

1 passed  0 skipped  0 failed
```

**Step 2: If the card lands in `failed/` instead of `done/`**

Debug with:
```sh
# Run the adapter directly (bypass dispatcher)
td=$(mktemp -d)
mkdir -p $td/output
echo "Write hello to output/result.md" > $td/prompt.md
OLLAMA_MODEL=qwen2.5:7b nu --no-config-file adapters/ollama-local.nu $td $td/prompt.md $td/stdout.log $td/stderr.log
echo "Exit: $?"
cat $td/stderr.log
cat $td/stdout.log
```

Common issues:
- `ollama not reachable` → start ollama: `ollama serve`
- `model not found` → `ollama pull qwen2.5:7b`
- Exit 75 → model name mismatch — check `ollama list`

**Step 3: Fix any bugs found, re-run**

If ollama-local.nu needs fixes (e.g. `output/result.md` not written for plain text responses), fix in `adapters/ollama-local.nu`.

The adapter already has a `mkdir output` and a structured JSON extraction path. For plain text responses with no `{"files": [...]}` block, `output/result.md` is NOT written by the current adapter — this is a bug. If found:

Add to `ollama-local.nu` after the JSON extraction block (around line 103):

```nushell
    # Fallback: if no structured output, write stdout as result.md
    if not ($"($workdir)/output/result.md" | path exists) {
        mkdir $"($workdir)/output"
        open --raw $stdout_abs | save --force $"($workdir)/output/result.md"
    }
```

**Step 4: Commit**

```sh
jj describe -m "fix: ollama-local.nu write stdout as result.md fallback when no structured JSON output"
```

---

### Task 4: Run claude end-to-end test

Claude CLI is installed at `/opt/homebrew/bin/claude`. This makes a real API call.

**Step 1: Run the claude test**

```sh
nu --no-config-file scripts/test-real-adapters.nu --adapter claude --timeout 120
```

Expected:
```
✓ claude
  claude: output/result.md = 'hello from claude'

1 passed  0 skipped  0 failed
```

**Step 2: If the card lands in `failed/`**

Check dispatcher logs:
```sh
# Run adapter directly to see raw output
td=$(mktemp -d)
mkdir -p $td/output
echo "Write the text 'hello from claude' to output/result.md" > $td/prompt.md
touch $td/stdout.log $td/stderr.log
nu --no-config-file adapters/claude.nu $td $td/prompt.md $td/stdout.log $td/stderr.log
echo "Exit: $?"
cat $td/stderr.log | head -20
ls $td/output/
```

Note: `claude.nu` captures stdout to `stdout.log` (the raw JSON output from `--output-format json`). The claude CLI writes result files directly to the workdir — `output/result.md` must be created by the claude session itself (via tool use), not by the adapter.

The prompt must instruct claude to create the file. If `output/result.md` is not being written, the prompt needs to be more explicit:

```
Create a file at output/result.md containing exactly the text: hello from claude

Use the Write tool to create this file. Do not do anything else.
```

**Step 3: Fix prompt in test script if needed, re-run**

**Step 4: Commit**

```sh
jj describe -m "test: claude adapter end-to-end verified — card reaches done/ with output/result.md"
```

---

### Task 5: Run codex end-to-end test

Codex CLI is installed at `/opt/homebrew/bin/codex`.

**Step 1: Run the codex test**

```sh
nu --no-config-file scripts/test-real-adapters.nu --adapter codex --timeout 120
```

Expected:
```
✓ codex
  codex: output/result.md = 'hello from codex'

1 passed  0 skipped  0 failed
```

**Step 2: If the card lands in `failed/`**

Run adapter directly:
```sh
td=$(mktemp -d)
mkdir -p $td/output
echo "Write the text 'hello from codex' to output/result.md. Use mkdir -p output first." > $td/prompt.md
touch $td/stdout.log $td/stderr.log
nu --no-config-file adapters/codex.nu $td $td/prompt.md $td/stdout.log $td/stderr.log
echo "Exit: $?"
cat $td/stderr.log | head -20
ls $td/output/
```

The codex adapter runs `codex exec --dangerously-bypass-approvals-and-sandbox` — it has full disk access. If it exits non-zero, check stderr for auth errors (OPENAI_API_KEY may not be set).

**Step 3: If OPENAI_API_KEY missing, mark codex as skip**

In `run_adapter_test`, update the codex availability check:

```nushell
"codex"  => (
    (which codex | length) > 0
    and ("OPENAI_API_KEY" in $env)
)
```

**Step 4: Run all three together**

```sh
nu --no-config-file scripts/test-real-adapters.nu
```

Expected (codex skipped if no API key):
```
✓ claude
✓ ollama
⊘ codex — skipped (tool not available)

2 passed  1 skipped  0 failed
```

**Step 5: Commit**

```sh
jj describe -m "test: all three adapters validated end-to-end (claude, ollama, codex)"
```

---

### Task 6: Final check

```sh
make check
nu --no-config-file scripts/test-real-adapters.nu
```

Both must pass.

---

## Notes on what this validates vs. what mock tested

| What | Mock tests | Real adapter tests |
|------|-----------|-------------------|
| State machine (pending→running→done) | ✓ | ✓ |
| Concurrent dispatcher race | ✓ | ✓ |
| Watcher latency | ✓ | ✓ |
| Tool is actually invoked | ✗ | ✓ |
| LLM generates output | ✗ | ✓ |
| `output/result.md` written | ✗ | ✓ |
| Adapter exit-code mapping | ✓ (unit) | ✓ (real) |
| Rate-limit/error handling | ✓ (unit) | ✓ (real, if triggered) |
