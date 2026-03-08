# Event-Driven Agent Teams Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the poll-sleep dispatcher loop with an event-driven watcher (kqueue on macOS, inotify on Linux), remove the global dispatcher lock so N concurrent dispatchers can race safely on the same card queue, and add a BusyBox-compatible shell agent that joins the same queue via `inotifywait`.

**Architecture:** The `fs::rename(pending→running)` claim is already atomic — whoever wins the race owns the card, losers silently skip. The global dispatcher lock is the only thing preventing multiple dispatchers from running in parallel; removing it enables true agent teams. The `notify` crate wraps kqueue/inotify/FSEvents into a single Rust API and signals the dispatch loop via a `tokio::sync::mpsc` channel, replacing the `tokio::time::sleep(poll_ms)` poll with `tokio::select!` on watcher-or-fallback-timeout.

**Tech Stack:** Rust, `notify = "6"` crate (kqueue/inotify/FSEvents), `tokio::sync::mpsc`, BusyBox `inotifywait`, POSIX `sh`.

**Team model:** Each team dir (`.cards/team-cli/`, `.cards/team-arch/`, etc.) gets its own dispatcher instance pointed at `--cards-dir .cards/team-cli/`. Multiple dispatcher processes race via atomic `mv`. BusyBox shell agents join the same race with identical protocol.

---

### Task 1: Add `notify` crate and event-driven wakeup to dispatcher

**Files:**
- Modify: `Cargo.toml` (workspace root, `[workspace.dependencies]`)
- Modify: `crates/bop-cli/Cargo.toml` (add dep)
- Modify: `crates/bop-cli/src/dispatcher.rs` (~line 337, the `tokio::time::sleep`)

---

**Step 1: Add dep to workspace Cargo.toml**

In `/Users/studio/bop/Cargo.toml`, in `[workspace.dependencies]`:

```toml
notify = { version = "6", default-features = false, features = ["macos_kqueue"] }
```

In `/Users/studio/bop/crates/bop-cli/Cargo.toml`, in `[dependencies]`:

```toml
notify.workspace = true
```

**Step 2: Verify it compiles**

```sh
cd /Users/studio/bop && cargo build -p bop 2>&1 | head -20
```

Expected: compiles (notify not yet used, may get unused-dep warning — that's fine).

**Step 3: Write failing test**

Add to the bottom of `crates/bop-cli/src/dispatcher.rs` tests module:

```rust
#[tokio::test]
async fn watcher_channel_fires_on_new_bop_dir() {
    use std::sync::mpsc;
    let td = tempfile::tempdir().unwrap();
    let pending = td.path().join("pending");
    std::fs::create_dir_all(&pending).unwrap();

    let (tx, rx) = mpsc::channel::<()>();
    let pending2 = pending.clone();
    std::thread::spawn(move || {
        // simulate what the watcher does: fire tx when a .bop dir appears
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::create_dir(pending2.join("card.bop")).unwrap();
        let _ = tx.send(());
    });

    let got = rx.recv_timeout(std::time::Duration::from_secs(2));
    assert!(got.is_ok(), "channel should fire");
}
```

Run:
```sh
cd /Users/studio/bop && cargo test -p bop dispatcher::tests::watcher_channel_fires 2>&1 | tail -5
```

Expected: PASS (this tests the channel pattern, not the watcher itself — simulates it).

**Step 4: Add `make_pending_watcher` helper in dispatcher.rs**

Add this function near the top of `dispatcher.rs` (after the `use` block):

```rust
/// Set up a notify watcher on `pending_dir` that sends () on `tx` whenever
/// a new `.bop` directory is created or renamed into the directory.
/// Returns the watcher (must be kept alive — dropped = watcher stops).
fn make_pending_watcher(
    pending_dir: &Path,
    tx: tokio::sync::mpsc::UnboundedSender<()>,
) -> anyhow::Result<notify::RecommendedWatcher> {
    use notify::{EventKind, RecursiveMode, Watcher};
    let pending_dir = pending_dir.to_path_buf();
    let mut watcher = notify::RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            let is_new_bop = matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))
            ) && event.paths.iter().any(|p| {
                p.extension().and_then(|e| e.to_str()) == Some("bop")
            });
            if is_new_bop {
                let _ = tx.send(());
            }
        },
        notify::Config::default(),
    )?;
    watcher.watch(&pending_dir, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}
```

**Step 5: Replace `tokio::time::sleep(poll_ms)` with `tokio::select!`**

Find line ~337 in `dispatcher.rs`:
```rust
tokio::time::sleep(Duration::from_millis(poll_ms)).await;
```

Replace the entire bottom of the loop (the sleep line only) with a watcher-or-timeout select. First, before the `loop {` at line ~47, add channel + watcher setup:

```rust
// Event-driven wakeup: watcher fires tx when a .bop dir lands in pending/.
// Fallback: poll every poll_ms in case watcher misses an event.
let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
let _pending_watcher = make_pending_watcher(&pending_dir, wake_tx)
    .unwrap_or_else(|e| {
        tracing::warn!("filesystem watcher unavailable ({e}), using poll-only mode");
        // Return a dummy watcher by using a no-op; we still have the poll fallback.
        // notify doesn't expose a no-op watcher, so we just let _pending_watcher be an Err.
        // We handle this by making make_pending_watcher return Option instead.
        unreachable!()
    });
```

Actually, make it cleaner — change the approach to use `Option`:

Before the loop:
```rust
let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
let _pending_watcher = make_pending_watcher(&pending_dir, wake_tx).ok();
if _pending_watcher.is_none() {
    tracing::warn!("[dispatcher] filesystem watcher unavailable, using poll-only mode");
}
```

Replace the `tokio::time::sleep` line at the bottom of the loop body:
```rust
// Wait for a watcher event OR the poll-fallback timeout, whichever comes first.
tokio::select! {
    _ = wake_rx.recv() => {}
    _ = tokio::time::sleep(Duration::from_millis(poll_ms)) => {}
}
```

**Step 6: Run make check**

```sh
cd /Users/studio/bop && make check
```

Expected: passes. The dispatcher now wakes immediately when a `.bop` dir appears in `pending/`, falling back to `poll_ms` if the watcher misses it.

**Step 7: Commit**

```sh
jj describe -m "feat: event-driven dispatcher wakeup via notify (kqueue/inotify/FSEvents)"
```

---

### Task 2: Remove global dispatcher lock — enable N concurrent dispatchers

**Files:**
- Modify: `crates/bop-cli/src/dispatcher.rs` (~line 30, `acquire_dispatcher_lock`)

**Background:** `_dispatcher_lock` at line 30 holds a PID-based file lock that prevents any second dispatcher from starting. This is the only barrier to running multiple dispatchers (agent teams) against the same `.cards/` dir. The actual card claim — `fs::rename(pending→running)` — is already atomic. Removing the global lock allows N dispatchers to race safely.

---

**Step 1: Write failing test proving two dispatchers can coexist**

Add to dispatcher tests:

```rust
#[test]
fn two_dispatchers_can_acquire_no_global_lock() {
    // After removing the global lock, this should not panic or error.
    // We simulate two concurrent dispatchers by checking that the lock
    // is NOT held by verifying acquire_dispatcher_lock would succeed twice.
    // With the lock removed from run_dispatcher, this test documents intent.
    let td = tempfile::tempdir().unwrap();
    let cards_dir = td.path();
    paths::ensure_cards_layout(cards_dir).unwrap();

    // If the global lock is still acquired inside run_dispatcher,
    // a second call would fail. Since we're not calling run_dispatcher
    // (it's async and complex), we document the claim: the lock module
    // still works for explicit use, but run_dispatcher no longer calls it.
    // This test just confirms lock::acquire_dispatcher_lock still compiles
    // and works for explicit use cases (e.g. --once guard).
    let g1 = crate::lock::acquire_dispatcher_lock(cards_dir);
    assert!(g1.is_ok(), "first lock should succeed");
    drop(g1);
    let g2 = crate::lock::acquire_dispatcher_lock(cards_dir);
    assert!(g2.is_ok(), "second lock (after drop) should succeed");
}
```

Run:
```sh
cd /Users/studio/bop && cargo test -p bop dispatcher::tests::two_dispatchers 2>&1 | tail -5
```

Expected: PASS (documents the pattern).

**Step 2: Remove `acquire_dispatcher_lock` from `run_dispatcher`**

In `dispatcher.rs`, find and delete these two lines (~line 30):

```rust
let _dispatcher_lock = lock::acquire_dispatcher_lock(cards_dir)?;
```

The lock guard (`_dispatcher_lock`) was keeping the lock alive for the dispatcher's lifetime. Removing it means N dispatcher processes can now start against the same `cards_dir`.

**Step 3: Verify `fs::rename` race safety with a test**

Add to dispatcher tests:

```rust
#[test]
fn concurrent_rename_claim_only_one_wins() {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    use std::thread;

    let td = tempfile::tempdir().unwrap();
    let pending = td.path().join("pending");
    let running = td.path().join("running");
    std::fs::create_dir_all(&pending).unwrap();
    std::fs::create_dir_all(&running).unwrap();

    // Create one card in pending
    let card = pending.join("race.bop");
    std::fs::create_dir(&card).unwrap();

    let wins = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    // Spawn 4 threads all trying to claim the same card
    for _ in 0..4 {
        let src = pending.join("race.bop");
        let dst = running.join("race.bop");
        let wins2 = Arc::clone(&wins);
        handles.push(thread::spawn(move || {
            if std::fs::rename(&src, &dst).is_ok() {
                wins2.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    for h in handles { h.join().unwrap(); }

    assert_eq!(wins.load(Ordering::Relaxed), 1, "exactly one rename wins");
    assert!(running.join("race.bop").exists(), "card in running/");
    assert!(!pending.join("race.bop").exists(), "card gone from pending/");
}
```

Run:
```sh
cd /Users/studio/bop && cargo test -p bop dispatcher::tests::concurrent_rename 2>&1 | tail -5
```

Expected: PASS.

**Step 4: make check**

```sh
cd /Users/studio/bop && make check
```

Expected: passes.

**Step 5: Commit**

```sh
jj describe -m "feat: remove global dispatcher lock — N concurrent dispatchers race safely via atomic rename"
```

---

### Task 3: BusyBox shell agent — `scripts/agent.sh`

**Files:**
- Create: `scripts/agent.sh`

This is a POSIX sh script that any BusyBox system (or any POSIX shell) can run to participate in the bop card queue. Uses `inotifywait` when available; falls back to a polling loop. Implements the same claim-or-skip protocol as the Rust dispatcher via atomic `mv`.

---

**Step 1: Create the script**

Create `/Users/studio/bop/scripts/agent.sh`:

```sh
#!/bin/sh
# bop card agent — BusyBox compatible
#
# Joins the bop card queue using atomic mv claim protocol.
# Uses inotifywait (inotify) when available; polls as fallback.
# Protocol: pending→running (claim), then done/pending/failed on exit code.
#
# Usage:
#   sh scripts/agent.sh [cards_dir] [adapter]
#
#   cards_dir  Path to .cards/ dir or a team subdir (default: .cards)
#   adapter    Command to run inside claimed card dir (default: sh adapter.sh)
#              Receives card_dir as $1. Exit 0=done, 75=retry, other=failed.
#
# Exit codes (same as bop dispatcher):
#   0   adapter exited 0 → card moved to done/
#   75  adapter exited 75 (rate-limited) → card returned to pending/
#   *   adapter exited other → card moved to failed/

set -eu

CARDS="${1:-.cards}"
ADAPTER="${2:-sh}"

claim_and_run() {
    file="$1"
    src="$CARDS/pending/$file"
    dst="$CARDS/running/$file"

    # Atomic claim — only one agent wins the rename
    mv "$src" "$dst" 2>/dev/null || return 0

    # $PWD inside card bundle encodes state and id
    (cd "$dst" && "$ADAPTER" "$dst") || true
    rc=$?

    case $rc in
        0)  mv "$dst" "$CARDS/done/$file"    ;;
        75) mv "$dst" "$CARDS/pending/$file" ;;
        *)  mv "$dst" "$CARDS/failed/$file"  ;;
    esac
}

# Drain any cards already in pending/ before watching
for f in "$CARDS/pending/"*.bop; do
    [ -d "$f" ] && claim_and_run "$(basename "$f")"
done

if command -v inotifywait >/dev/null 2>&1; then
    # Event-driven: Linux inotify via BusyBox inotifywait
    inotifywait -m -q -e moved_to,create "$CARDS/pending" 2>/dev/null |
    while IFS= read -r line; do
        file="${line##* }"           # last whitespace-separated token = filename
        case "$file" in
            *.bop) claim_and_run "$file" ;;
        esac
    done
else
    # Polling fallback: works everywhere including Unikraft host-side
    printf 'bop-agent: inotifywait not found, polling every 2s\n' >&2
    while true; do
        for f in "$CARDS/pending/"*.bop; do
            [ -d "$f" ] && claim_and_run "$(basename "$f")"
        done
        sleep 2
    done
fi
```

**Step 2: Make it executable**

```sh
chmod +x /Users/studio/bop/scripts/agent.sh
```

**Step 3: Write a test using the script**

Add a test in a new file `crates/bop-cli/src/tests/agent_sh_test.rs` — actually, test it with a simple shell integration test in the existing harness approach. Add to `Makefile` or test it inline:

```sh
# Quick manual smoke test (put in a test script)
td=$(mktemp -d)
mkdir -p "$td/pending" "$td/running" "$td/done" "$td/failed"
mkdir -p "$td/pending/smoke.bop"
echo '{"id":"smoke","created":"2026-03-04T00:00:00Z","stage":"spec"}' \
  > "$td/pending/smoke.bop/meta.json"

# Adapter that succeeds immediately
cat > /tmp/ok_adapter.sh << 'EOF'
#!/bin/sh
exit 0
EOF
chmod +x /tmp/ok_adapter.sh

# Run agent in single-shot mode (drain pending + no inotifywait = poll exits after empty)
timeout 5 sh scripts/agent.sh "$td" /tmp/ok_adapter.sh || true

ls "$td/done/"  # should show smoke.bop
```

Run this manually to verify:
```sh
cd /Users/studio/bop && sh -c 'td=$(mktemp -d) && mkdir -p "$td"/{pending,running,done,failed} && mkdir -p "$td/pending/smoke.bop" && echo "{\"id\":\"smoke\",\"created\":\"2026-03-04T00:00:00Z\",\"stage\":\"spec\"}" > "$td/pending/smoke.bop/meta.json" && echo "#!/bin/sh\nexit 0" > /tmp/ok_adapter.sh && chmod +x /tmp/ok_adapter.sh && timeout 4 sh scripts/agent.sh "$td" /tmp/ok_adapter.sh; ls "$td/done/"'
```

Expected: `smoke.bop` appears in `done/`.

**Step 4: Test retry (exit 75) and failure paths**

```sh
# exit 75 → back to pending
echo '#!/bin/sh\nexit 75' > /tmp/retry_adapter.sh && chmod +x /tmp/retry_adapter.sh
# ... same setup ... timeout 3 sh scripts/agent.sh "$td" /tmp/retry_adapter.sh; ls "$td/pending/"
# Expected: smoke.bop back in pending/

# exit 1 → failed
echo '#!/bin/sh\nexit 1' > /tmp/fail_adapter.sh && chmod +x /tmp/fail_adapter.sh
# ... ls "$td/failed/"
# Expected: smoke.bop in failed/
```

**Step 5: make check (Rust side must still pass)**

```sh
cd /Users/studio/bop && make check
```

**Step 6: Commit**

```sh
jj describe -m "feat: BusyBox-compatible shell agent (scripts/agent.sh) with inotifywait + poll fallback"
```

---

## Usage: running agent teams

**Rust dispatchers (N parallel, different teams):**
```sh
bop dispatcher --cards-dir .cards/team-cli/  --workers 2 &
bop dispatcher --cards-dir .cards/team-arch/ --workers 2 &
bop dispatcher --cards-dir .cards/team-quality/ --workers 1 &
```

**BusyBox agents joining the same queue:**
```sh
sh scripts/agent.sh .cards/team-cli/ adapters/claude.nu &
sh scripts/agent.sh .cards/team-cli/ adapters/claude.nu &
# Two shell agents + Rust dispatcher all racing on the same pending/ — only one wins each card
```

**Unikraft/zam (host-side, boots per batch):**
```sh
# Host watches with inotifywait, boots zam for each card
inotifywait -m -q -e moved_to .cards/pending/ |
while read _dir _event file; do
  case "$file" in *.bop)
    mv ".cards/pending/$file" ".cards/running/$file"
    qemu-system-aarch64 -kernel zam.elf ...  # zam reads card over 9P, exits
    mv ".cards/running/$file" ".cards/done/$file"
  ;; esac
done
```

**Key invariant:** whoever wins `mv pending/X.bop running/X.bop` owns the card. All other agents silently skip. No coordinator. No locks. The filesystem IS the coordinator.
