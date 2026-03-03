use anyhow::Context;
use jobcard_core::write_meta;
use std::fs;
use std::path::Path;

use crate::paths;

fn glyph_rank(g: &str) -> (&'static str, u32) {
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

fn glyph_suit(g: &str) -> &'static str {
    let cp = g.chars().next().map(|c| c as u32).unwrap_or(0);
    match (cp >> 4) & 0xF {
        0xA => "♠ complexity",
        0xB => "♥ effort",
        0xC => "♦ risk",
        0xD => "♣ value",
        _ => "? unknown",
    }
}

fn is_joker(g: &str) -> bool {
    g.chars()
        .next()
        .map(jobcard_core::cardchars::is_joker)
        .unwrap_or(false)
}

pub fn cmd_poker_open(root: &Path, id: &str) -> anyhow::Result<()> {
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let mut meta = jobcard_core::read_meta(&card)?;
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
    let mut meta = jobcard_core::read_meta(&card)?;
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
    let mut meta = jobcard_core::read_meta(&card)?;
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
    let meta = jobcard_core::read_meta(&card)?;
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
    let mut meta = jobcard_core::read_meta(&card)?;
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

    // Rename dir: {old-glyph}-{id}.jobcard → {glyph}-{id}.jobcard
    let new_name = format!("{}-{}.jobcard", glyph, id);
    let new_card = card.parent().unwrap_or(root).join(&new_name);
    if new_card != card {
        fs::rename(&card, &new_card)?;
        println!("  renamed → {}", new_name);
    }

    println!("∴ Consensus: {glyph} — {rank_label} of {suit} — {pts}pt");
    println!("  Committed to {id}/meta.json");
    Ok(())
}
