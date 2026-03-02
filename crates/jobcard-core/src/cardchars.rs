//! Card character assignment: Team -> suit mapping, glyph allocation, and
//! used-glyph collection from the `.cards/` filesystem.

use std::collections::HashSet;
use std::path::Path;

/// Teams map 1:1 to playing-card suits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Team {
    /// Spades (U+1F0A0 base, BMP token U+2660)
    Cli,
    /// Hearts (U+1F0B0 base, BMP token U+2665)
    Arch,
    /// Diamonds (U+1F0C0 base, BMP token U+2666)
    Quality,
    /// Clubs (U+1F0D0 base, BMP token U+2663)
    Platform,
}

impl Team {
    /// SMP base codepoint for this suit (the "back" card, rank 0).
    const fn smp_base(self) -> u32 {
        match self {
            Team::Cli => 0x1F0A0,
            Team::Arch => 0x1F0B0,
            Team::Quality => 0x1F0C0,
            Team::Platform => 0x1F0D0,
        }
    }

    /// BMP suit symbol used as the filename-safe token.
    const fn bmp_token(self) -> char {
        match self {
            Team::Cli => '\u{2660}',      // BLACK SPADE SUIT
            Team::Arch => '\u{2665}',     // BLACK HEART SUIT
            Team::Quality => '\u{2666}',  // BLACK DIAMOND SUIT
            Team::Platform => '\u{2663}', // BLACK CLUB SUIT
        }
    }
}

// ── Special cards (not auto-assigned) ────────────────────────────────────────

/// Back of card — placeholder glyph before assignment.
pub const CARD_BACK: char = '\u{1F0A0}'; // 🂠

/// Jokers — emergency / wildcard / needs-breakdown.
pub const RED_JOKER: char = '\u{1F0BF}';   // 🂿
pub const BLACK_JOKER: char = '\u{1F0CF}';  // 🃏
pub const WHITE_JOKER: char = '\u{1F0DF}';  // 🃟

/// Trump cards (U+1F0E0..U+1F0F5) — cross-team escalation.
/// Fool(0), then I(1)..XXI(21). 22 cards total.
pub const TRUMP_FOOL: char = '\u{1F0E0}';   // 🃠
pub const TRUMP_MAX: char = '\u{1F0F5}';    // 🃵
/// Number of trump cards in the Unicode block.
pub const TRUMP_COUNT: u32 = 22;

/// All joker codepoints for quick membership checks.
pub const JOKERS: [char; 3] = [RED_JOKER, BLACK_JOKER, WHITE_JOKER];

/// Check whether a glyph char is a joker.
pub fn is_joker(ch: char) -> bool {
    JOKERS.contains(&ch)
}

/// Check whether a glyph char is a trump card (Fool through XXI).
pub fn is_trump(ch: char) -> bool {
    let cp = ch as u32;
    cp >= TRUMP_FOOL as u32 && cp <= TRUMP_MAX as u32
}

// ── Trump BMP tokens ────────────────────────────────────────────────────────

/// Return the (glyph, token) pair for a trump card by rank (0=Fool .. 21=World).
///
/// Glyph is the SMP character (U+1F0E0+rank).
/// Token is a circled number in the BMP for terminal/filename display:
///   rank 0  → ⓪ U+24FF
///   rank 1–20 → ①–⑳ U+2460–U+2473
///   rank 21 → ㉑ U+3251
pub fn trump_glyph_and_token(rank: u32) -> Option<(char, char)> {
    if rank > 21 {
        return None;
    }
    let glyph = char::from_u32(0x1F0E0 + rank)?;
    let token = match rank {
        0 => '\u{24FF}',                              // ⓪
        1..=20 => char::from_u32(0x245F + rank)?,     // ①–⑳
        21 => '\u{3251}',                              // ㉑
        _ => unreachable!(),
    };
    Some((glyph, token))
}

// ── Auto-assignment ──────────────────────────────────────────────────────────

/// Return the first unused (glyph, token) pair for `team`.
///
/// Walks ranks Ace(1) through King(14). Returns `None` when all 14 slots
/// in the suit are occupied.
pub fn next_glyph(team: Team, used: &HashSet<char>) -> Option<(String, String)> {
    let base = team.smp_base();
    for rank in 1..=14u32 {
        let cp = base + rank;
        let ch = char::from_u32(cp)?;
        if !used.contains(&ch) {
            return Some((String::from(ch), String::from(team.bmp_token())));
        }
    }
    None
}

/// Detect the team from directory path components.
///
/// Scans for `team-cli`, `team-arch`, `team-quality`, `team-platform`.
/// Defaults to `Team::Cli` when no team directory is found.
pub fn team_from_path(path: &Path) -> Team {
    for component in path.components() {
        if let std::path::Component::Normal(os) = component {
            if let Some(s) = os.to_str() {
                match s {
                    "team-cli" => return Team::Cli,
                    "team-arch" => return Team::Arch,
                    "team-quality" => return Team::Quality,
                    "team-platform" => return Team::Platform,
                    _ => {}
                }
            }
        }
    }
    Team::Cli
}

/// Scan all card state directories under `cards_root` and collect the first
/// character of every `glyph` field found in `meta.json`.
///
/// Scans both top-level state dirs (`pending/`, `running/`, `done/`, `failed/`)
/// and team-scoped dirs (`team-*/pending/`, etc.).
pub fn collect_used_glyphs(cards_root: &Path) -> HashSet<char> {
    let mut used = HashSet::new();
    let state_dirs = ["pending", "running", "done", "failed"];

    // Collect from a single state directory
    let mut scan_state_dir = |dir: &Path| {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir()
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(".jobcard"))
                {
                    if let Ok(meta) = crate::read_meta(&path) {
                        if let Some(glyph) = &meta.glyph {
                            if let Some(ch) = glyph.chars().next() {
                                used.insert(ch);
                            }
                        }
                    }
                }
            }
        }
    };

    // Top-level state dirs
    for state in &state_dirs {
        scan_state_dir(&cards_root.join(state));
    }

    // Team-scoped dirs: team-*/state/
    if let Ok(entries) = std::fs::read_dir(cards_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("team-") {
                        for state in &state_dirs {
                            scan_state_dir(&path.join(state));
                        }
                    }
                }
            }
        }
    }

    used
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_glyph_returns_ace_first() {
        let used = HashSet::new();
        let (glyph, token) = next_glyph(Team::Cli, &used).unwrap();
        assert_eq!(glyph, "\u{1F0A1}");
        assert_eq!(token, "\u{2660}");
    }

    #[test]
    fn next_glyph_skips_used() {
        let mut used = HashSet::new();
        used.insert('\u{1F0A1}');
        let (glyph, _) = next_glyph(Team::Cli, &used).unwrap();
        assert_eq!(glyph, "\u{1F0A2}");
    }

    #[test]
    fn next_glyph_returns_none_when_full() {
        let mut used = HashSet::new();
        for i in 1..=14 {
            used.insert(char::from_u32(0x1F0A0 + i).unwrap());
        }
        assert!(next_glyph(Team::Cli, &used).is_none());
    }

    #[test]
    fn next_glyph_hearts_for_arch() {
        let used = HashSet::new();
        let (glyph, token) = next_glyph(Team::Arch, &used).unwrap();
        assert_eq!(glyph, "\u{1F0B1}");
        assert_eq!(token, "\u{2665}");
    }

    #[test]
    fn next_glyph_diamonds_for_quality() {
        let used = HashSet::new();
        let (glyph, token) = next_glyph(Team::Quality, &used).unwrap();
        assert_eq!(glyph, "\u{1F0C1}");
        assert_eq!(token, "\u{2666}");
    }

    #[test]
    fn next_glyph_clubs_for_platform() {
        let used = HashSet::new();
        let (glyph, token) = next_glyph(Team::Platform, &used).unwrap();
        assert_eq!(glyph, "\u{1F0D1}");
        assert_eq!(token, "\u{2663}");
    }

    #[test]
    fn team_from_path_detects_team_dirs() {
        assert_eq!(
            team_from_path(Path::new("/x/.cards/team-cli/pending/foo.jobcard")),
            Team::Cli
        );
        assert_eq!(
            team_from_path(Path::new("/x/.cards/team-arch/pending/foo.jobcard")),
            Team::Arch
        );
        assert_eq!(
            team_from_path(Path::new("/x/.cards/team-quality/pending/foo.jobcard")),
            Team::Quality
        );
        assert_eq!(
            team_from_path(Path::new("/x/.cards/team-platform/pending/foo.jobcard")),
            Team::Platform
        );
    }

    #[test]
    fn team_from_path_defaults_to_cli() {
        assert_eq!(
            team_from_path(Path::new("/x/.cards/pending/foo.jobcard")),
            Team::Cli
        );
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

    #[test]
    fn collect_used_glyphs_from_temp_cards() {
        let tmp = tempfile::tempdir().unwrap();
        let cards_root = tmp.path();

        // Create pending/test1.jobcard/meta.json with a glyph
        let card_dir = cards_root.join("pending").join("test1.jobcard");
        std::fs::create_dir_all(&card_dir).unwrap();
        let meta = crate::Meta {
            id: "test1".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            glyph: Some("\u{1F0A1}".into()),
            ..Default::default()
        };
        crate::write_meta(&card_dir, &meta).unwrap();

        // Create team-arch/running/test2.jobcard/meta.json
        let card_dir2 = cards_root
            .join("team-arch")
            .join("running")
            .join("test2.jobcard");
        std::fs::create_dir_all(&card_dir2).unwrap();
        let meta2 = crate::Meta {
            id: "test2".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            glyph: Some("\u{1F0B3}".into()),
            ..Default::default()
        };
        crate::write_meta(&card_dir2, &meta2).unwrap();

        let used = collect_used_glyphs(cards_root);
        assert!(used.contains(&'\u{1F0A1}'));
        assert!(used.contains(&'\u{1F0B3}'));
        assert_eq!(used.len(), 2);
    }

    #[test]
    fn card_back_is_correct_codepoint() {
        assert_eq!(CARD_BACK as u32, 0x1F0A0);
    }

    #[test]
    fn joker_detection() {
        assert!(is_joker(RED_JOKER));
        assert!(is_joker(BLACK_JOKER));
        assert!(is_joker(WHITE_JOKER));
        assert!(!is_joker('\u{1F0A1}')); // Ace of Spades is not a joker
    }

    #[test]
    fn trump_detection() {
        assert!(is_trump(TRUMP_FOOL));
        assert!(is_trump(TRUMP_MAX));
        assert!(is_trump('\u{1F0E5}')); // Trump V
        assert!(!is_trump('\u{1F0A1}')); // Ace of Spades is not a trump
        assert!(!is_trump('\u{1F0F6}')); // Past the end of trumps
    }

    #[test]
    fn trump_glyph_and_token_fool() {
        let (glyph, token) = trump_glyph_and_token(0).unwrap();
        assert_eq!(glyph, TRUMP_FOOL);
        assert_eq!(token, '\u{24FF}'); // ⓪
    }

    #[test]
    fn trump_glyph_and_token_magician() {
        let (glyph, token) = trump_glyph_and_token(1).unwrap();
        assert_eq!(glyph as u32, 0x1F0E1);
        assert_eq!(token, '\u{2460}'); // ①
    }

    #[test]
    fn trump_glyph_and_token_world() {
        let (glyph, token) = trump_glyph_and_token(21).unwrap();
        assert_eq!(glyph, TRUMP_MAX);
        assert_eq!(token, '\u{3251}'); // ㉑
    }

    #[test]
    fn trump_glyph_and_token_all_22() {
        for rank in 0..22 {
            let (glyph, token) = trump_glyph_and_token(rank).unwrap();
            assert_eq!(glyph as u32, 0x1F0E0 + rank);
            // All tokens must be BMP (< U+10000)
            assert!((token as u32) < 0x10000, "rank {rank} token not BMP");
        }
    }

    #[test]
    fn trump_glyph_and_token_out_of_range() {
        assert!(trump_glyph_and_token(22).is_none());
        assert!(trump_glyph_and_token(100).is_none());
    }

    #[test]
    fn collect_used_glyphs_skips_no_glyph() {
        let tmp = tempfile::tempdir().unwrap();
        let cards_root = tmp.path();

        let card_dir = cards_root.join("done").join("noglph.jobcard");
        std::fs::create_dir_all(&card_dir).unwrap();
        let meta = crate::Meta {
            id: "noglph".into(),
            created: chrono::Utc::now(),
            stage: "implement".into(),
            glyph: None,
            ..Default::default()
        };
        crate::write_meta(&card_dir, &meta).unwrap();

        let used = collect_used_glyphs(cards_root);
        assert!(used.is_empty());
    }
}
