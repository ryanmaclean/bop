use anyhow::Context;
use bop_core::write_meta;
use std::fs;
use std::path::Path;

use crate::paths;

pub(crate) fn glyph_rank(g: &str) -> (&'static str, u32) {
    let cp = g.chars().next().map(|c| c as u32).unwrap_or(0);
    match cp & 0xF {
        1 => ("Ace", 1),
        2 => ("2", 2),
        3 => ("3", 3),
        4 => ("4", 4),
        5 => ("5", 5),
        6 => ("6", 6),
        7 => ("7", 7),
        8 => ("8", 8),
        9 => ("9", 9),
        10 => ("10", 10),
        11 => ("Jack", 13),
        12 => ("Knight", 20),
        13 => ("Queen", 21),
        14 => ("King", 40),
        _ => ("Joker", 0),
    }
}

pub(crate) fn glyph_suit(g: &str) -> &'static str {
    let cp = g.chars().next().map(|c| c as u32).unwrap_or(0);
    match (cp >> 4) & 0xF {
        0xA => "♠ complexity",
        0xB => "♥ effort",
        0xC => "♦ risk",
        0xD => "♣ value",
        _ => "? unknown",
    }
}

pub(crate) fn is_joker(g: &str) -> bool {
    g.chars()
        .next()
        .map(bop_core::cardchars::is_joker)
        .unwrap_or(false)
}

pub fn cmd_poker_open(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let mut meta = bop_core::read_meta(&card)?;
    if meta.poker_round.as_deref() == Some("open") {
        println!("Round already open for {}", id);
        return Ok(());
    }
    meta.poker_round = Some("open".into());
    meta.estimates.clear();
    write_meta(&card, &meta)?;
    println!("🂠  Poker round opened for {id}. Submit with: bop poker submit {id}");
    Ok(())
}

pub fn cmd_poker_submit(
    root: &Path,
    id: &str,
    glyph: Option<&str>,
    name: Option<&str>,
) -> anyhow::Result<()> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let mut meta = bop_core::read_meta(&card)?;
    if meta.poker_round.as_deref() != Some("open") {
        anyhow::bail!("no open round for {id}. Run: bop poker open {id}");
    }
    let participant = name
        .map(str::to_owned)
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "anonymous".into());

    let chosen = if let Some(g) = glyph {
        g.to_owned()
    } else {
        // Simple fallback: prompt for glyph when no TTY picker available
        eprint!("Enter glyph (e.g. 🂻): ");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        line.trim().to_owned()
    };

    if chosen.is_empty() {
        anyhow::bail!("no glyph provided");
    }
    meta.estimates.insert(participant.clone(), chosen);
    write_meta(&card, &meta)?;
    println!("🂠  {participant} submitted (face-down until reveal)");
    Ok(())
}

pub fn cmd_poker_reveal(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let mut meta = bop_core::read_meta(&card)?;
    if meta.poker_round.as_deref() != Some("open") {
        anyhow::bail!("no open round for {id}");
    }
    meta.poker_round = Some("revealed".into());
    write_meta(&card, &meta)?;

    println!("\n  Estimates for {id}:\n");
    let mut joker_players: Vec<String> = vec![];
    let mut points: Vec<u32> = vec![];

    for (participant, glyph) in &meta.estimates {
        if is_joker(glyph) {
            joker_players.push(participant.clone());
            println!("  {participant:<12} {glyph}  Joker — needs breakdown");
        } else {
            let (rank_label, pts) = glyph_rank(glyph);
            let suit = glyph_suit(glyph);
            println!("  {participant:<12} {glyph}  {rank_label} of {suit} — {pts}pt");
            points.push(pts);
        }
    }

    if !joker_players.is_empty() {
        println!(
            "\n  ⊘ {} played 🃏 — break down the card first",
            joker_players.join(", ")
        );
        return Ok(());
    }

    if points.len() > 1 {
        let mut sorted = points.clone();
        sorted.sort_unstable();
        let median = sorted[sorted.len() / 2];
        let spread = sorted.last().unwrap_or(&0) - sorted.first().unwrap_or(&0);
        println!("\n  Spread: {spread}pt  Median: {median}pt");
        for (participant, glyph) in &meta.estimates {
            let (rank_label, pts) = glyph_rank(glyph);
            if median > 0 && (pts < median / 2 || pts > median * 2) {
                println!("  ⚡ outlier: {participant} ({glyph} {rank_label}  {pts}pt vs median {median}pt)");
            }
        }
    }
    println!("\n  Run: bop poker consensus {id} <glyph>");
    Ok(())
}

pub fn cmd_poker_status(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let meta = bop_core::read_meta(&card)?;
    match meta.poker_round.as_deref() {
        Some("open") => {
            println!("Round: open  ({} submitted)", meta.estimates.len());
            for name in meta.estimates.keys() {
                println!("  🂠 {name}");
            }
        }
        Some("revealed") => {
            println!("Round: revealed");
            for (name, glyph) in &meta.estimates {
                println!("  {glyph} {name}");
            }
        }
        _ => println!("No active round for {id}"),
    }
    Ok(())
}

pub fn cmd_poker_consensus(root: &Path, id: &str, glyph: &str) -> anyhow::Result<()> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let mut meta = bop_core::read_meta(&card)?;
    if meta.poker_round.is_none() {
        anyhow::bail!("no active round for {id}");
    }
    if is_joker(glyph) {
        println!("⊘ {glyph} is a Joker — cannot commit. Break down the card first.");
        return Ok(());
    }
    let (rank_label, pts) = glyph_rank(glyph);
    let suit = glyph_suit(glyph);
    meta.glyph = Some(glyph.to_owned());
    meta.poker_round = None;
    meta.estimates.clear();
    write_meta(&card, &meta)?;

    // Rename dir: {old-glyph}-{id}.bop → {glyph}-{id}.bop
    let new_name = format!("{}-{}.bop", glyph, id);
    let new_card = card.parent().unwrap_or(root).join(&new_name);
    if new_card != card {
        fs::rename(&card, &new_card)?;
        println!("  renamed → {}", new_name);
    }

    println!("∴ Consensus: {glyph} — {rank_label} of {suit} — {pts}pt");
    println!("  Committed to {id}/meta.json");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ── glyph_rank ──────────────────────────────────────────────────────
    #[test]
    fn glyph_rank_ace() {
        // U+1F0A1 = Ace of Spades — low nibble 1
        let (label, pts) = glyph_rank("\u{1F0A1}");
        assert_eq!(label, "Ace");
        assert_eq!(pts, 1);
    }

    #[test]
    fn glyph_rank_king() {
        // U+1F0AE = King of Spades — low nibble 0xE = 14
        let (label, pts) = glyph_rank("\u{1F0AE}");
        assert_eq!(label, "King");
        assert_eq!(pts, 40);
    }

    #[test]
    fn glyph_rank_jack() {
        // U+1F0AB = Jack of Spades — low nibble 0xB = 11
        let (label, pts) = glyph_rank("\u{1F0AB}");
        assert_eq!(label, "Jack");
        assert_eq!(pts, 13);
    }

    #[test]
    fn glyph_rank_five() {
        // U+1F0A5 = 5 of Spades — low nibble 5
        let (label, pts) = glyph_rank("\u{1F0A5}");
        assert_eq!(label, "5");
        assert_eq!(pts, 5);
    }

    #[test]
    fn glyph_rank_joker() {
        // U+1F0CF = Black Joker — low nibble 0xF
        let (label, pts) = glyph_rank("\u{1F0CF}");
        assert_eq!(label, "Joker");
        assert_eq!(pts, 0);
    }

    // ── glyph_suit ──────────────────────────────────────────────────────
    #[test]
    fn glyph_suit_spades() {
        // U+1F0A1 — high nibble of low byte: 0xA → spades
        assert_eq!(glyph_suit("\u{1F0A1}"), "♠ complexity");
    }

    #[test]
    fn glyph_suit_hearts() {
        // U+1F0B1 — high nibble: 0xB → hearts
        assert_eq!(glyph_suit("\u{1F0B1}"), "♥ effort");
    }

    #[test]
    fn glyph_suit_diamonds() {
        // U+1F0C1 — high nibble: 0xC → diamonds
        assert_eq!(glyph_suit("\u{1F0C1}"), "♦ risk");
    }

    #[test]
    fn glyph_suit_clubs() {
        // U+1F0D1 — high nibble: 0xD → clubs
        assert_eq!(glyph_suit("\u{1F0D1}"), "♣ value");
    }

    // ── is_joker ────────────────────────────────────────────────────────
    #[test]
    fn is_joker_true_for_joker() {
        assert!(is_joker("\u{1F0DF}")); // 🃟
    }

    #[test]
    fn is_joker_false_for_normal_card() {
        assert!(!is_joker("\u{1F0A1}")); // Ace of Spades
    }

    // ── cmd_poker_open ──────────────────────────────────────────────────
    fn setup_card(td: &std::path::Path, id: &str) -> std::path::PathBuf {
        let card_dir = td.join("pending").join(format!("{}.bop", id));
        fs::create_dir_all(&card_dir).unwrap();
        let meta = bop_core::Meta {
            id: id.into(),
            stage: "implement".into(),
            ..Default::default()
        };
        bop_core::write_meta(&card_dir, &meta).unwrap();
        card_dir
    }

    #[test]
    fn poker_open_sets_round() {
        let td = tempdir().unwrap();
        let card_dir = setup_card(td.path(), "test-card");
        cmd_poker_open(td.path(), "test-card").unwrap();
        let meta = bop_core::read_meta(&card_dir).unwrap();
        assert_eq!(meta.poker_round.as_deref(), Some("open"));
        assert!(meta.estimates.is_empty());
    }

    #[test]
    fn poker_open_idempotent() {
        let td = tempdir().unwrap();
        setup_card(td.path(), "test-card");
        cmd_poker_open(td.path(), "test-card").unwrap();
        // Opening again should succeed (already open)
        cmd_poker_open(td.path(), "test-card").unwrap();
    }

    // ── cmd_poker_submit ────────────────────────────────────────────────
    #[test]
    fn poker_submit_records_estimate() {
        let td = tempdir().unwrap();
        let card_dir = setup_card(td.path(), "test-card");
        cmd_poker_open(td.path(), "test-card").unwrap();
        cmd_poker_submit(td.path(), "test-card", Some("\u{1F0A5}"), Some("alice")).unwrap();
        let meta = bop_core::read_meta(&card_dir).unwrap();
        assert_eq!(
            meta.estimates.get("alice").map(|s| s.as_str()),
            Some("\u{1F0A5}")
        );
    }

    #[test]
    fn poker_submit_fails_without_open_round() {
        let td = tempdir().unwrap();
        setup_card(td.path(), "test-card");
        let result = cmd_poker_submit(td.path(), "test-card", Some("\u{1F0A5}"), Some("alice"));
        assert!(result.is_err());
    }

    // ── cmd_poker_reveal ────────────────────────────────────────────────
    #[test]
    fn poker_reveal_sets_revealed() {
        let td = tempdir().unwrap();
        let card_dir = setup_card(td.path(), "test-card");
        cmd_poker_open(td.path(), "test-card").unwrap();
        cmd_poker_submit(td.path(), "test-card", Some("\u{1F0A5}"), Some("alice")).unwrap();
        cmd_poker_reveal(td.path(), "test-card").unwrap();
        let meta = bop_core::read_meta(&card_dir).unwrap();
        assert_eq!(meta.poker_round.as_deref(), Some("revealed"));
    }

    #[test]
    fn poker_reveal_fails_without_open_round() {
        let td = tempdir().unwrap();
        setup_card(td.path(), "test-card");
        let result = cmd_poker_reveal(td.path(), "test-card");
        assert!(result.is_err());
    }

    // ── cmd_poker_consensus ─────────────────────────────────────────────
    #[test]
    fn poker_consensus_commits_glyph_and_clears_round() {
        let td = tempdir().unwrap();
        setup_card(td.path(), "test-card");
        cmd_poker_open(td.path(), "test-card").unwrap();
        cmd_poker_submit(td.path(), "test-card", Some("\u{1F0A5}"), Some("alice")).unwrap();
        cmd_poker_consensus(td.path(), "test-card", "\u{1F0A5}").unwrap();
        // Card got renamed to {glyph}-test-card.bop
        let new_card = td
            .path()
            .join("pending")
            .join("\u{1F0A5}-test-card.bop");
        assert!(new_card.exists());
        let meta = bop_core::read_meta(&new_card).unwrap();
        assert_eq!(meta.glyph.as_deref(), Some("\u{1F0A5}"));
        assert!(meta.poker_round.is_none());
        assert!(meta.estimates.is_empty());
    }

    #[test]
    fn poker_consensus_rejects_joker() {
        let td = tempdir().unwrap();
        setup_card(td.path(), "test-card");
        cmd_poker_open(td.path(), "test-card").unwrap();
        // Submitting a joker as consensus should succeed but not commit
        cmd_poker_consensus(td.path(), "test-card", "\u{1F0DF}").unwrap();
        // Card should still be in original location with round still open
        let card_dir = td.path().join("pending").join("test-card.bop");
        let meta = bop_core::read_meta(&card_dir).unwrap();
        assert_eq!(meta.poker_round.as_deref(), Some("open"));
    }
}
