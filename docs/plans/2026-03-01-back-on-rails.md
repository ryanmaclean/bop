# Back on Rails: Trim Bloat + Ship Missing Pieces

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Get the project back to the plan. Remove scope-creep features that aren't in the design doc, fix what's half-built, ship the missing macOS-native differentiators.

**Architecture:** The project has a working Rust dispatcher/CLI and a working Xcode project (JobCardHost.app + gtfs.appex Quick Look extension) that builds and registers the UTI. The Rust side has ~2000 lines of scope creep (REST API, TUI dashboard, memory system, realtime module) that need to go. The macOS side needs Quick Look actually tested end-to-end, a Spotlight MDImporter, and the launchd plists validated.

**Tech Stack:** Rust (cargo workspace), Swift (Xcode project), launchd plists, macOS system APIs

---

## Current state (verified)

**Working:**
- Rust dispatcher with provider failover, orphan reaping (85 tests pass)
- 7 adapter shell scripts (claude, codex, goose, aider, opencode, ollama-local, mock)
- Merge gate with acceptance criteria + git merge --no-ff
- CLI: init, new, status, validate, dispatcher, merge-gate, retry, kill, logs, inspect
- Xcode project builds (`xcodebuild -scheme JobCardHost` succeeds)
- UTI registered with Launch Services (`com.yourorg.jobcard` active, exported, trusted)
- Quick Look extension registered (`pluginkit` shows `com.gtfs.JobCardHost.QuickLook`)
- Spotlight finds `.jobcard` bundles by content type
- launchd plists exist for dispatcher and merge-gate
- APFS COW cloning (cp -c / --reflink=auto) with platform abstraction

**Scope creep (not in plan, remove):**
- REST API server (axum, tower, utoipa, OpenAPI) — ~600 lines in main.rs + 5 deps
- TUI dashboard (ratatui, crossterm) — ~400 lines in main.rs + 2 deps
- Memory system (memory list/get/set/delete) — ~200 lines in main.rs
- `create --from-description` command — ~150 lines in main.rs
- `realtime` module in jobcard-core — 732 lines, not wired in
- `config` module CLI commands (config get/set) — ~100 lines in main.rs
- Worktree CLI subcommands (worktree list/create/clean) — ~200 lines in main.rs
- Provider CLI subcommands (providers list/add/remove/status) — ~250 lines in main.rs

**Half-built / needs verification:**
- Quick Look preview — extension is registered but hasn't been tested on a real card in Finder
- No Spotlight MDImporter (Spotlight sees the bundles but can't search by stage/agent)
- launchd plists have hardcoded paths, not templated
- Shell completions don't exist
- No `make check` — the Makefile exists but need to verify it runs test+clippy+fmt

**Duplicate Swift code:**
- `JobCardQuickLook/` (Swift Package at repo root) — duplicate of code in `macos/`
- `JobCardType/` (Swift Package at repo root) — stub, does nothing
- Loose Swift files at repo root: `JobCardPreviewProvider.swift`, `patch.swift`, `test_app.swift`, `test_ql.swift`

---

## Part A: Cut the bloat

### Task 1: Remove realtime module from jobcard-core

The `realtime` module (732 lines) is a standalone feed-validation thing that is not part of the plan and is not wired into anything.

**Files:**
- Delete: `crates/jobcard-core/src/realtime.rs`
- Modify: `crates/jobcard-core/src/lib.rs` (remove `pub mod realtime;`)

**Step 1: Remove the module declaration**

In `crates/jobcard-core/src/lib.rs`, delete the line `pub mod realtime;`

**Step 2: Delete the file**

```bash
rm crates/jobcard-core/src/realtime.rs
```

**Step 3: Remove `--realtime` flag from validate command**

In `crates/jc/src/main.rs`, find the `Validate` variant and remove the `realtime: bool` field and any code that uses it. The `validate` command itself stays (it's in the plan), just the realtime feed validation goes.

**Step 4: Run tests**

```bash
cargo test
```

Expected: All remaining tests pass. The realtime tests are gone.

**Step 5: Commit**

```bash
git add -A && git commit -m "remove: realtime module (not in plan, not wired in)"
```

### Task 2: Remove REST API server

The `serve` command and all axum/tower/utoipa code is not in the plan.

**Files:**
- Modify: `crates/jc/src/main.rs` (remove Serve command, all REST handlers, SSE endpoint, OpenAPI)
- Modify: `crates/jc/Cargo.toml` (remove axum, tower, tower-http, utoipa, async-stream deps)
- Delete: `crates/jc/tests/rest_api_harness.rs`
- Delete: `crates/jc/tests/web_ui_harness.rs`

**Step 1: Remove the Serve command variant and all REST/API code**

Remove from main.rs:
- `Command::Serve { .. }` enum variant
- `cmd_serve()` function and all helper types (`ApiJob`, `CreateJobRequest`, etc.)
- All `#[derive(ToSchema)]` annotations
- The `#[derive(OpenApi)]` struct
- All axum handler functions
- The SSE log streaming endpoint
- The embedded HTML for `--ui`

**Step 2: Remove dependencies from Cargo.toml**

Remove from `crates/jc/Cargo.toml`:
```
axum = "0.7"
tower = "0.5"
tower-http = { version = "0.6", features = ["cors"] }
utoipa = { version = "5", features = ["chrono"] }
async-stream = "0.3"
```

Also remove from dev-dependencies:
```
reqwest = { version = "0.12", features = ["blocking", "json"] }
```

**Step 3: Delete test files**

```bash
rm crates/jc/tests/rest_api_harness.rs
rm crates/jc/tests/web_ui_harness.rs
```

**Step 4: Remove unused imports from main.rs**

Remove: `use axum::*`, `use tower::*`, `use tower_http::*`, `use utoipa::*`, `use async_stream::*`, `use tokio::net::TcpListener`

**Step 5: Build and test**

```bash
cargo build && cargo test
```

Expected: Compiles. All remaining tests pass. Build should be noticeably faster without axum dependency tree.

**Step 6: Commit**

```bash
git add -A && git commit -m "remove: REST API server (not in plan)"
```

### Task 3: Remove TUI dashboard

The `dashboard` command with ratatui is not in the plan. `jc status` is the dashboard — it's `ls`.

**Files:**
- Modify: `crates/jc/src/main.rs` (remove Dashboard command and all ratatui code)
- Modify: `crates/jc/Cargo.toml` (remove ratatui, crossterm deps)
- Delete: `crates/jc/tests/dashboard_harness.rs`

**Step 1: Remove Dashboard command and all TUI code**

Remove from main.rs:
- `Command::Dashboard` variant
- `cmd_dashboard()` function
- All ratatui rendering code (`draw_dashboard`, etc.)
- `DashboardState` struct

**Step 2: Remove dependencies**

Remove from `crates/jc/Cargo.toml`:
```
crossterm = "0.29"
ratatui = "0.29"
```

**Step 3: Delete test file**

```bash
rm crates/jc/tests/dashboard_harness.rs
```

**Step 4: Remove unused imports**

Remove: `use crossterm::*`, `use ratatui::*`

**Step 5: Build and test**

```bash
cargo build && cargo test
```

**Step 6: Commit**

```bash
git add -A && git commit -m "remove: TUI dashboard (not in plan, jc status is the dashboard)"
```

### Task 4: Remove memory system

The memory CLI commands and in-dispatcher memory injection are not in the plan.

**Files:**
- Modify: `crates/jc/src/main.rs` (remove Memory command, memory functions)
- Delete: `crates/jc/tests/memory_harness.rs`

**Step 1: Remove Memory command and all memory code**

Remove from main.rs:
- `MemoryCommand` enum and `Command::Memory` variant
- `cmd_memory_list()`, `cmd_memory_get()`, `cmd_memory_set()`, `cmd_memory_delete()`
- Memory injection in dispatcher (the part that reads `memory/` dir and passes `{{memory}}` to prompts)
- Memory-out merge after adapter runs

**Step 2: Delete test file**

```bash
rm crates/jc/tests/memory_harness.rs
```

**Step 3: Build and test**

```bash
cargo build && cargo test
```

**Step 4: Commit**

```bash
git add -A && git commit -m "remove: memory system (not in plan)"
```

### Task 5: Remove create-from-description command

Not in plan. `jc new <template> <id>` is the creation path.

**Files:**
- Modify: `crates/jc/src/main.rs` (remove Create command)
- Delete: `crates/jc/tests/create_from_description_harness.rs`

**Step 1: Remove Create command**

Remove `Command::Create { .. }` variant and `cmd_create_from_description()` function.

**Step 2: Delete test file**

```bash
rm crates/jc/tests/create_from_description_harness.rs
```

**Step 3: Build and test**

```bash
cargo build && cargo test
```

**Step 4: Commit**

```bash
git add -A && git commit -m "remove: create-from-description (not in plan)"
```

### Task 6: Remove config CLI subcommands

The `config get/set` CLI isn't in the plan. The config module in jobcard-core can stay (it's used internally), but the CLI subcommands go.

**Files:**
- Modify: `crates/jc/src/main.rs` (remove Config command)
- Delete: `crates/jc/tests/config_cmd.rs`

**Step 1: Remove Config command**

Remove `ConfigCommand` enum, `Command::Config` variant, and the match arm.

**Step 2: Delete test file**

```bash
rm crates/jc/tests/config_cmd.rs
```

**Step 3: Build and test**

```bash
cargo build && cargo test
```

**Step 4: Commit**

```bash
git add -A && git commit -m "remove: config CLI subcommands (not in plan)"
```

### Task 7: Remove worktree and providers CLI subcommands

The `worktree list/create/clean` and `providers list/add/remove/status` CLI commands are not in the plan. Worktree creation happens in the dispatcher. Provider management is done by editing `providers.json`.

**Files:**
- Modify: `crates/jc/src/main.rs` (remove Worktree and Providers commands)
- Delete: `crates/jc/tests/worktree_harness.rs`
- Delete: `crates/jc/tests/providers_harness.rs`

**Step 1: Remove both command groups**

Remove `WorktreeAction` enum, `ProvidersCommand` enum, `Command::Worktree` variant, `Command::Providers` variant, and all associated functions.

**Step 2: Delete test files**

```bash
rm crates/jc/tests/worktree_harness.rs
rm crates/jc/tests/providers_harness.rs
```

**Step 3: Build and test**

```bash
cargo build && cargo test
```

**Step 4: Commit**

```bash
git add -A && git commit -m "remove: worktree and providers CLI subcommands (not in plan)"
```

### Task 8: Clean up loose Swift files and duplicate packages

**Files:**
- Delete: `JobCardQuickLook/` (duplicate of code in `macos/`)
- Delete: `JobCardType/` (stub that does nothing)
- Delete: `JobCardPreviewProvider.swift` (loose file at repo root)
- Delete: `patch.swift`, `test_app.swift`, `test_ql.swift` (loose test files)
- Delete: `patch_pbxproj.rb`, `patch_pbxproj2.rb`, `patch_pbxproj_remove_provider.rb`, `fix_pbx.rb` (build hacks)
- Delete: `test_app/`, `test_ql/`, `test_data/` (loose test dirs if they exist)
- Delete: `CLEANUP_SUMMARY.md`, `COMPLETION_PLAN.md`, `FOLDER_STRUCTURE.md`, `IDEATION.md`, `IMPLEMENTATION_STATUS.md`, `MODEL_LOOKUP.md`, `NEW_PLAN.md`, `PORTABLE_ADAPTERS.md`, `ROADMAP.md` (slop docs)

**Step 1: Remove all of the above**

```bash
rm -rf JobCardQuickLook/ JobCardType/
rm -f JobCardPreviewProvider.swift patch.swift test_app.swift test_ql.swift
rm -f patch_pbxproj.rb patch_pbxproj2.rb patch_pbxproj_remove_provider.rb fix_pbx.rb
rm -rf test_app/ test_ql/ test_data/ test.jobcard
rm -f CLEANUP_SUMMARY.md COMPLETION_PLAN.md FOLDER_STRUCTURE.md IDEATION.md
rm -f IMPLEMENTATION_STATUS.md MODEL_LOOKUP.md NEW_PLAN.md PORTABLE_ADAPTERS.md ROADMAP.md
```

**Step 2: Verify Xcode project still builds**

```bash
cd macos && xcodebuild -scheme JobCardHost -configuration Debug build
```

**Step 3: Verify Rust still builds and tests pass**

```bash
cargo build && cargo test
```

**Step 4: Commit**

```bash
git add -A && git commit -m "remove: duplicate Swift packages, loose files, slop docs"
```

### Task 9: Final trim verification

**Step 1: Count lines**

```bash
wc -l crates/jc/src/main.rs crates/jobcard-core/src/*.rs
```

Expected: main.rs should be ~2000-2500 lines (down from 4825). jobcard-core should be ~430 lines (down from 6168 - the 732-line realtime module is gone).

**Step 2: Run full check**

```bash
cargo test && cargo clippy -- -D warnings && cargo fmt --check
```

**Step 3: Commit any fmt fixes**

```bash
cargo fmt && git add -A && git commit -m "fmt: post-trim formatting"
```

---

## Part B: Verify + fix the macOS integration

### Task 10: Test Quick Look end-to-end

The extension is registered but we need to verify it actually renders a preview.

**Step 1: Create a test card with real data**

```bash
cd /Users/studio/gtfs
cp -c .cards/templates/implement.jobcard /tmp/test-ql.jobcard
```

Edit `/tmp/test-ql.jobcard/meta.json` to have realistic data (non-placeholder id, stage, etc.)

**Step 2: Open Quick Look in Finder**

```bash
qlmanage -p /tmp/test-ql.jobcard
```

If the HTML preview renders showing the card title, stage, agent, and acceptance criteria, Quick Look works.

**Step 3: If it doesn't work**

Check Console.app for Quick Look errors:
```bash
log show --predicate 'subsystem == "com.apple.quicklook"' --last 5m
```

Debug the PreviewProvider.swift — the likely issue is the `UTType.html.identifier` return type or the `QLPreviewReply` API usage. The extension in `macos/JobCardQuickLook/PreviewProvider.swift` returns HTML data, which is the right approach.

**Step 4: Document result**

If working: commit any fixes. If not: file the specific error for the next task.

### Task 11: Validate and template the launchd plists

**Files:**
- Modify: `launchd/com.yourorg.jobcard.dispatcher.plist`
- Modify: `launchd/com.yourorg.jobcard.merge-gate.plist`

**Step 1: Check both plists parse**

```bash
plutil -lint launchd/com.yourorg.jobcard.dispatcher.plist
plutil -lint launchd/com.yourorg.jobcard.merge-gate.plist
```

Expected: OK for both.

**Step 2: Fix hardcoded paths**

The dispatcher plist has `/Users/studio/gtfs` hardcoded. Replace with a comment explaining the user should edit, or add a `jc install-launchd` command that generates the plist with the right paths (this IS in the plan's spirit — the launchd plist is a plan deliverable).

**Step 3: Test loading the dispatcher**

```bash
launchctl load launchd/com.yourorg.jobcard.dispatcher.plist
launchctl list | grep jobcard
launchctl unload launchd/com.yourorg.jobcard.dispatcher.plist
```

**Step 4: Commit**

```bash
git add -A && git commit -m "fix: launchd plists validated and paths documented"
```

### Task 12: Add shell completions

The plan says: "Shell completions (bash, zsh, fish)". Clap generates these for free.

**Files:**
- Modify: `crates/jc/src/main.rs` (add completions generation command or build.rs)
- Create: `completions/jc.bash`
- Create: `completions/jc.zsh`
- Create: `completions/jc.fish`

**Step 1: Add clap_complete dependency**

In `crates/jc/Cargo.toml`:
```toml
clap_complete = "4"
```

**Step 2: Add a Completions command**

```rust
Command::Completions { shell: clap_complete::Shell },
```

And handle it:
```rust
Command::Completions { shell } => {
    clap_complete::generate(shell, &mut Cli::command(), "jc", &mut std::io::stdout());
    Ok(())
}
```

**Step 3: Generate and commit the completion files**

```bash
cargo build
./target/debug/jc completions bash > completions/jc.bash
./target/debug/jc completions zsh > completions/jc.zsh
./target/debug/jc completions fish > completions/jc.fish
git add -A && git commit -m "feat: add shell completions (bash, zsh, fish)"
```

---

## Summary

After this plan:
- main.rs drops from ~4825 to ~2000-2500 lines
- 5 heavyweight deps removed (axum, tower, ratatui, utoipa, reqwest)
- Build time drops significantly
- All plan deliverables verified working (UTI, Quick Look, launchd, completions)
- Repo is clean of slop docs and duplicate files
- What remains matches the plan: dispatcher, adapters, merge gate, CLI, macOS integration
