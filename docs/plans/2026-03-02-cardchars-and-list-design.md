# CARDCHARS Module + `bop list` Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Auto-assign unique playing-card glyphs to every new card and surface them via `bop list`.

**Architecture:** New `cardchars.rs` module in `bop-core` with a Team enum, const lookup tables for 4 suits x 14 ranks, and a `next_glyph()` function that scans existing cards to avoid collisions. The CLI wires this into `create_card()` and adds a `bop list` subcommand.

**Tech Stack:** Rust, serde, clap (existing workspace deps)

---

### Task 1: Add `token` field to `Meta` struct

The `token` field exists in meta.json files but is missing from the Rust struct.
Serde silently drops it on deserialize and loses it on re-serialize. Fix this first
so the rest of the work has a field to write to.

**Files:**
- Modify: `crates/bop-core/src/lib.rs:69-196` (Meta struct)

**Step 1: Write the failing test**

Add to the `tests` module in `crates/bop-core/src/lib.rs`:

```rust
#[test]
fn meta_token_field_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let m = Meta {
        id: "tok1".into(),
        created: chrono::Utc::now(),
        stage: "implement".into(),
        glyph: Some("\u{1F0AB}".into()),
        token: Some("\u{2660}".into()),
        ..Default::default()
    };
    write_meta(dir.path(), &m).unwrap();
    let back = read_meta(dir.path()).unwrap();
    assert_eq!(back.token.as_deref(), Some("\u{2660}"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p bop-core meta_token_field_round_trips`
Expected: FAIL — `Meta` has no field named `token`

**Step 3: Write minimal implementation**

Add to the `Meta` struct, right after the `glyph` field (line ~81):

```rust
    /// BMP-safe token for terminal, filenames, pane titles.
    /// Suit symbol: ♠♥♦♣ for CLI/Arch/Quality/Platform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
```

Update the `Default`-using sites: the two `Meta { ... }` literals in `main.rs`
(lines ~905 and ~997) already use `..Default::default()` or explicit `None` — the
new field gets `None` via `Default`. Verify the template-init block at line ~882
also works (it uses explicit field listing, so add `token: None,` there).

**Step 4: Run test to verify it passes**

Run: `cargo test -p bop-core meta_token_field_round_trips`
Expected: PASS

**Step 5: Run full suite**

Run: `cargo test`
Expected: All existing tests still pass

**Step 6: Commit**

```
git add crates/bop-core/src/lib.rs
git commit -m "feat: add token field to Meta struct

Existing meta.json files contain a token field that was silently
dropped on deserialize. Now it round-trips correctly."
```

---

### Task 2: Create `cardchars.rs` — Team enum and lookup tables

**Files:**
- Create: `crates/bop-core/src/cardchars.rs`
- Modify: `crates/bop-core/src/lib.rs:1-3` (add `pub mod cardchars;`)

**Step 1: Write the failing tests**

Create `crates/bop-core/src/cardchars.rs` with tests only:

```rust
use std::collections::HashSet;
use std::path::Path;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_glyph_returns_ace_first() {
        let used = HashSet::new();
        let (glyph, token) = next_glyph(Team::Cli, &used).unwrap();
        // Ace of Spades = U+1F0A1
        assert_eq!(glyph, "\u{1F0A1}");
        assert_eq!(token, "\u{2660}");
    }

    #[test]
    fn next_glyph_skips_used() {
        let mut used = HashSet::new();
        used.insert('\u{1F0A1}'); // Ace of Spades taken
        let (glyph, _) = next_glyph(Team::Cli, &used).unwrap();
        // 2 of Spades = U+1F0A2
        assert_eq!(glyph, "\u{1F0A2}");
    }

    #[test]
    fn next_glyph_returns_none_when_full() {
        let mut used = HashSet::new();
        // Fill all 14 spade ranks: U+1F0A1..U+1F0AE
        for i in 1..=14 {
            used.insert(char::from_u32(0x1F0A0 + i).unwrap());
        }
        assert!(next_glyph(Team::Cli, &used).is_none());
    }

    #[test]
    fn next_glyph_hearts_for_arch() {
        let used = HashSet::new();
        let (glyph, token) = next_glyph(Team::Arch, &used).unwrap();
        assert_eq!(glyph, "\u{1F0B1}"); // Ace of Hearts
        assert_eq!(token, "\u{2665}");   // ♥
    }

    #[test]
    fn next_glyph_diamonds_for_quality() {
        let used = HashSet::new();
        let (glyph, token) = next_glyph(Team::Quality, &used).unwrap();
        assert_eq!(glyph, "\u{1F0C1}"); // Ace of Diamonds
        assert_eq!(token, "\u{2666}");   // ♦
    }

    #[test]
    fn next_glyph_clubs_for_platform() {
        let used = HashSet::new();
        let (glyph, token) = next_glyph(Team::Platform, &used).unwrap();
        assert_eq!(glyph, "\u{1F0D1}"); // Ace of Clubs
        assert_eq!(token, "\u{2663}");   // ♣
    }

    #[test]
    fn team_from_path_detects_team_dirs() {
        assert_eq!(team_from_path(Path::new("/x/.cards/team-cli/pending/foo.bop")), Team::Cli);
        assert_eq!(team_from_path(Path::new("/x/.cards/team-arch/pending/foo.bop")), Team::Arch);
        assert_eq!(team_from_path(Path::new("/x/.cards/team-quality/pending/foo.bop")), Team::Quality);
        assert_eq!(team_from_path(Path::new("/x/.cards/team-platform/pending/foo.bop")), Team::Platform);
    }

    #[test]
    fn team_from_path_defaults_to_cli() {
        assert_eq!(team_from_path(Path::new("/x/.cards/pending/foo.bop")), Team::Cli);
    }

    #[test]
    fn sequential_assignment_across_all_14() {
        let mut used = HashSet::new();
        for i in 0..14 {
            let (glyph, _) = next_glyph(Team::Cli, &used).unwrap();
            let ch = glyph.chars().next().unwrap();
            assert_eq!(ch as u32, 0x1F0A1 + i as u32);
            used.insert(ch);
        }
        assert!(next_glyph(Team::Cli, &used).is_none());
    }
}
```

**Step 2: Add module declaration**

In `crates/bop-core/src/lib.rs`, line 1, add:

```rust
pub mod cardchars;
```

**Step 3: Run tests to verify they fail**

Run: `cargo test -p bop-core -- cardchars`
Expected: FAIL — functions not defined

**Step 4: Write the implementation**

Fill in `crates/bop-core/src/cardchars.rs` above the tests:

```rust
use std::collections::HashSet;
use std::path::Path;

/// Team determines the card suit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Team {
    Cli,
    Arch,
    Quality,
    Platform,
}

/// Base codepoint for each suit's Ace (rank 1).
/// Spades=0x1F0A0, Hearts=0x1F0B0, Diamonds=0x1F0C0, Clubs=0x1F0D0.
/// Rank N is base + N (Ace=1, 2=2, ... 10=10, Jack=11, Knight=12, Queen=13, King=14).
impl Team {
    const fn suit_base(self) -> u32 {
        match self {
            Team::Cli => 0x1F0A0,
            Team::Arch => 0x1F0B0,
            Team::Quality => 0x1F0C0,
            Team::Platform => 0x1F0D0,
        }
    }

    /// BMP suit symbol for terminal/filename use.
    pub const fn token_char(self) -> char {
        match self {
            Team::Cli => '♠',
            Team::Arch => '♥',
            Team::Quality => '♦',
            Team::Platform => '♣',
        }
    }
}

/// Return the next unused (glyph, token) pair for a team.
///
/// Walks ranks 1..=14 (Ace through King) and returns the first
/// whose SMP codepoint is not in `used`. Returns `None` if the
/// suit is fully occupied.
pub fn next_glyph(team: Team, used: &HashSet<char>) -> Option<(String, String)> {
    let base = team.suit_base();
    for rank in 1..=14u32 {
        let cp = base + rank;
        if let Some(ch) = char::from_u32(cp) {
            if !used.contains(&ch) {
                return Some((ch.to_string(), team.token_char().to_string()));
            }
        }
    }
    None
}

/// Detect team from a card's filesystem path.
///
/// Looks for `team-cli`, `team-arch`, `team-quality`, `team-platform`
/// in any ancestor component. Defaults to `Cli` if none found.
pub fn team_from_path(card_path: &Path) -> Team {
    for component in card_path.components() {
        if let Some(s) = component.as_os_str().to_str() {
            match s {
                "team-cli" => return Team::Cli,
                "team-arch" => return Team::Arch,
                "team-quality" => return Team::Quality,
                "team-platform" => return Team::Platform,
                _ => {}
            }
        }
    }
    Team::Cli
}

/// Scan all card directories under `cards_root` and collect glyph chars in use.
///
/// Reads meta.json from every `.bop` dir found in any state directory
/// (pending, running, done, failed) and in team-*/state/ paths.
pub fn collect_used_glyphs(cards_root: &Path) -> HashSet<char> {
    let mut used = HashSet::new();
    let state_dirs = ["pending", "running", "done", "failed"];

    let mut scan = |dir: &Path| {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() && p.extension().and_then(|e| e.to_str()) == Some("bop") {
                    // "extension" won't work for ".bop" dirs — check file_name
                }
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if p.is_dir() && name.ends_with(".bop") {
                    if let Ok(meta) = crate::read_meta(&p) {
                        if let Some(g) = &meta.glyph {
                            if let Some(ch) = g.chars().next() {
                                used.insert(ch);
                            }
                        }
                    }
                }
            }
        }
    };

    // Root state dirs
    for state in &state_dirs {
        scan(&cards_root.join(state));
    }

    // Team state dirs
    if let Ok(entries) = std::fs::read_dir(cards_root) {
        for entry in entries.flatten() {
            let p = entry.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if p.is_dir() && name.starts_with("team-") {
                for state in &state_dirs {
                    scan(&p.join(state));
                }
            }
        }
    }

    used
}
```

**Step 5: Run tests to verify they pass**

Run: `cargo test -p bop-core -- cardchars`
Expected: All 9 tests PASS

**Step 6: Run clippy**

Run: `cargo clippy -p bop-core -- -D warnings`
Expected: No warnings

**Step 7: Commit**

```
git add crates/bop-core/src/cardchars.rs crates/bop-core/src/lib.rs
git commit -m "feat: add cardchars module with Team enum and glyph lookup

Sequential assignment of playing-card glyphs per team/suit.
Spades=CLI, Hearts=Arch, Diamonds=Quality, Clubs=Platform.
14 ranks per suit, collision-free via used-set scan."
```

---

### Task 3: Wire auto-assign into `create_card()`

**Files:**
- Modify: `crates/bop-cli/src/main.rs:30-36` (Command::New — add --team flag)
- Modify: `crates/bop-cli/src/main.rs:935-1035` (create_card function)
- Modify: `crates/bop-cli/src/main.rs:2060-2062` (Command::New match arm)

**Step 1: Add `--team` to `Command::New`**

Change `Command::New` (line ~33) from:
```rust
    New {
        template: String,
        id: String,
    },
```
to:
```rust
    New {
        template: String,
        id: String,
        /// Team for glyph suit assignment (cli, arch, quality, platform).
        /// Auto-detected from card directory if omitted.
        #[arg(long)]
        team: Option<String>,
    },
```

**Step 2: Update match arm**

Change the `Command::New` match (line ~2060) from:
```rust
        Command::New { template, id } => {
            create_card(&root, &template, &id, None)?;
            Ok(())
        }
```
to:
```rust
        Command::New { template, id, team } => {
            create_card(&root, &template, &id, None, team.as_deref())?;
            Ok(())
        }
```

**Step 3: Update `create_card` signature and add auto-assign**

Add `team_override: Option<&str>` parameter to `create_card` (line ~935).

After `meta.failure_reason = None;` (line ~1018), before `write_meta`, add:

```rust
    // Auto-assign glyph + token if not already set
    if meta.glyph.is_none() {
        use bop_core::cardchars::{self, Team};
        let team = match team_override {
            Some("cli") => Team::Cli,
            Some("arch") => Team::Arch,
            Some("quality") => Team::Quality,
            Some("platform") => Team::Platform,
            Some(other) => {
                eprintln!("warning: unknown team '{}', defaulting to cli", other);
                Team::Cli
            }
            None => cardchars::team_from_path(&card_dir),
        };
        let used = cardchars::collect_used_glyphs(cards_dir);
        if let Some((glyph, token)) = cardchars::next_glyph(team, &used) {
            meta.glyph = Some(glyph);
            meta.token = Some(token);
        } else {
            eprintln!("warning: suit full for {:?}, no glyph assigned", team);
        }
    }
```

Also update the other `create_card` call site — search for any other callers.

**Step 4: Update other callers of create_card**

Run: `grep -n 'create_card(' crates/bop-cli/src/main.rs` to find all call sites.
Each needs the new `team_override` param (pass `None` for internal callers like
stage-advance).

**Step 5: Also fix the `🂠` hardcode in card_dir path**

Line ~963 currently uses:
```rust
    let card_dir = cards_dir.join("pending").join(format!("🂠-{}.bop", id));
```

After glyph assignment, rename the card dir to use the assigned glyph instead
of `🂠`. But since the glyph is assigned *after* the dir is created, keep the
initial name and rename after meta write:

```rust
    // Rename card dir from placeholder to glyph-prefixed
    if let Some(ref g) = meta.glyph {
        let new_name = format!("{}-{}.bop", g, id);
        let new_dir = card_dir.parent().unwrap().join(&new_name);
        if !new_dir.exists() {
            std::fs::rename(&card_dir, &new_dir)?;
            return Ok(new_dir);
        }
    }
```

**Step 6: Run full test suite**

Run: `cargo test`
Expected: PASS (existing tests may need updating if they call create_card)

**Step 7: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

**Step 8: Commit**

```
git add crates/bop-cli/src/main.rs
git commit -m "feat: auto-assign glyph+token on card creation

New cards get a unique playing-card glyph based on team (suit)
with sequential rank assignment. --team flag overrides auto-detect.
Card directory is renamed from placeholder to glyph prefix."
```

---

### Task 4: Add `bop list` subcommand

**Files:**
- Modify: `crates/bop-cli/src/main.rs` (add Command::List, impl list_cards fn)

**Step 1: Add `Command::List` variant**

After `Command::Inspect` (line ~111), add:

```rust
    /// List cards with glyphs, stages, and progress.
    List {
        /// Filter by state: pending, running, done, failed, merged, all.
        #[arg(long, default_value = "active")]
        state: String,
    },
```

**Step 2: Write the list function**

```rust
fn list_cards(root: &Path, state_filter: &str) -> anyhow::Result<()> {
    let states: Vec<&str> = match state_filter {
        "all" => vec!["pending", "running", "done", "failed", "merged"],
        "active" => vec!["pending", "running", "done"],
        other => vec![other],
    };

    for state in &states {
        print_state_group(root, state, false)?;

        // Also check team-* directories
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let p = entry.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if p.is_dir() && name.starts_with("team-") {
                    print_state_group(&p, state, true)?;
                }
            }
        }
    }
    Ok(())
}

fn print_state_group(dir: &Path, state: &str, is_team: bool) -> anyhow::Result<()> {
    let state_dir = dir.join(state);
    if !state_dir.exists() {
        return Ok(());
    }

    let mut cards: Vec<(String, bop_core::Meta)> = Vec::new();
    if let Ok(entries) = fs::read_dir(&state_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Ok(meta) = bop_core::read_meta(&p) {
                    cards.push((p.to_string_lossy().to_string(), meta));
                }
            }
        }
    }

    let prefix = if is_team {
        let team_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("team");
        format!("{}/{}", team_name, state)
    } else {
        state.to_string()
    };

    println!("{} ({})", prefix, cards.len());
    for (_, meta) in &cards {
        let glyph = meta.glyph.as_deref().unwrap_or("  ");
        let token = meta.token.as_deref().unwrap_or(" ");
        let id = if meta.id.len() > 32 { &meta.id[..32] } else { &meta.id };
        let stage = &meta.stage;
        let pri = meta.priority.map(|p| format!("P{}", p)).unwrap_or_else(|| "--".into());
        let pct = meta.progress.unwrap_or(0);
        let filled = (pct as usize) / 13; // 8 chars = 100%
        let bar: String = (0..8).map(|i| if i < filled { '█' } else { '░' }).collect();
        let pct_str = if pct > 0 { format!("{}%", pct) } else { String::new() };
        println!("  {} {}  {:<32}  {:<10} {:<3} {} {}", glyph, token, id, stage, pri, bar, pct_str);
    }
    if cards.is_empty() {
        // Just show the header with count 0
    }
    println!();
    Ok(())
}
```

**Step 3: Wire into command dispatch**

Add to the main match:
```rust
        Command::List { state } => list_cards(&root, &state),
```

**Step 4: Build and test manually**

Run: `cargo build`
Run: `./target/debug/bop list`
Expected: Renders existing cards with their glyphs

**Step 5: Commit**

```
git add crates/bop-cli/src/main.rs
git commit -m "feat: add bop list command

Shows cards grouped by state with glyph, token, id, stage,
priority, and progress bar. --state flag filters (default: active).
Includes team-* subdirectories."
```

---

### Task 5: Replace `print_status_summary` with rich output

**Files:**
- Modify: `crates/bop-cli/src/main.rs:2008-2017` (`print_status_summary` function)

**Step 1: Replace the function body**

Change `print_status_summary` to call `list_cards`:

```rust
fn print_status_summary(root: &Path) -> anyhow::Result<()> {
    list_cards(root, "active")
}
```

**Step 2: Build and test**

Run: `cargo build`
Run: `./target/debug/bop status`
Expected: Shows the same rich listing as `bop list`

**Step 3: Commit**

```
git add crates/bop-cli/src/main.rs
git commit -m "feat: bop status (no-arg) now shows rich card listing

Replaces bare state counts with the same glyph+progress output
as bop list."
```

---

### Task 6: Final integration test and cleanup

**Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass

**Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: Clean

**Step 3: Check formatting**

Run: `cargo fmt --check`
Expected: No changes needed (run `cargo fmt` if needed)

**Step 4: Run make check**

Run: `make check`
Expected: All green

**Step 5: Manual smoke test**

```bash
# Create a new card and verify it gets a glyph
./target/debug/bop new implement test-glyph-auto
cat .cards/pending/*test-glyph-auto*/meta.json | grep -E '"glyph"|"token"'
# Expected: "glyph": "\uXXXX" (not null), "token": "♠"

# List cards
./target/debug/bop list
# Expected: shows cards with glyphs

# Create with --team override
./target/debug/bop new implement test-arch-card --team arch
cat .cards/pending/*test-arch-card*/meta.json | grep -E '"glyph"|"token"'
# Expected: "token": "♥" (Hearts/Arch)
```

**Step 6: Final commit if any fixups needed**

```
git add -A
git commit -m "chore: integration test fixups for cardchars"
```

---

## Future: Trump Cards as Agent Personas

The Unicode trump cards (U+1F0E0..U+1F0F5) map to Tarot major arcana.
These are ideal as agent persona glyphs — shown in Zellij pane titles,
`bop list` agent column, and dispatcher logs.

| Codepoint | Glyph | Tarot Name | Agent Role |
|-----------|-------|------------|------------|
| U+1F0E0 | 🃠 | The Fool | Explorer / brainstormer |
| U+1F0E1 | 🃡 | The Magician | Implementer |
| U+1F0E2 | 🃢 | The High Priestess | Spec reviewer |
| U+1F0E3 | 🃣 | The Empress | Code quality reviewer |
| U+1F0E4 | 🃤 | The Emperor | Architect / tech lead |
| U+1F0E5 | 🃥 | The Hierophant | Documentation writer |
| U+1F0E6 | 🃦 | The Lovers | Integration tester |
| U+1F0E7 | 🃧 | The Chariot | Dispatcher |
| U+1F0E8 | 🃨 | Strength | Debugger |
| U+1F0E9 | 🃩 | The Hermit | Deep researcher |
| U+1F0EA | 🃪 | Wheel of Fortune | Retry / failover handler |
| U+1F0EB | 🃫 | Justice | Policy checker |
| U+1F0EC | 🃬 | The Hanged Man | Blocked-task reviewer |
| U+1F0ED | 🃭 | Death | Dead code hunter / cleanup |
| U+1F0EE | 🃮 | Temperance | Refactorer (balance) |
| U+1F0EF | 🃯 | The Devil | Security auditor |
| U+1F0F0 | 🃰 | The Tower | Breaking change handler |
| U+1F0F1 | 🃱 | The Star | Performance optimizer |
| U+1F0F2 | 🃲 | The Moon | Edge case finder |
| U+1F0F3 | 🃳 | The Sun | Test writer |
| U+1F0F4 | 🃴 | Judgement | Final reviewer |
| U+1F0F5 | 🃵 | The World | Release manager |

### Implementation sketch (future PR)

1. Add `Persona` enum to `cardchars.rs` with `trump_glyph()` method
2. Dispatcher assigns persona to `agent_type` based on stage
3. `bop list` shows persona glyph in an agent column
4. Zellij pane title: `🃡 implement feat-auth` instead of `agent: implement`

Constants already exist in `cardchars.rs` (`TRUMP_FOOL` through `TRUMP_MAX`).
