use std::fs;
use std::path::Path;

pub fn cmd_events(
    root: &Path,
    card_filter: Option<&str>,
    json_mode: bool,
    limit: usize,
) -> anyhow::Result<()> {
    let events_path = root.join("events.jsonl");
    if !events_path.exists() {
        println!("no events yet — enable lineage with OPENLINEAGE_URL or .cards/hooks.toml");
        return Ok(());
    }

    let content = fs::read_to_string(&events_path)?;
    let events: Vec<jobcard_core::lineage::RunEvent> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|ev: &jobcard_core::lineage::RunEvent| {
            card_filter.is_none_or(|id| ev.job.name.contains(id))
        })
        .collect();

    let display: Vec<_> = events.iter().rev().take(limit).collect();

    if json_mode {
        for ev in display.iter().rev() {
            println!("{}", serde_json::to_string(ev).unwrap_or_default());
        }
    } else {
        println!(
            "{:<20} {:<10} {:<30} {:<8}",
            "TIME", "EVENT", "CARD", "RUN_ID"
        );
        for ev in display.iter().rev() {
            let time = ev.event_time.format("%Y-%m-%d %H:%M:%S");
            let event_type = format!("{:?}", ev.event_type);
            let card = if ev.job.name.len() > 28 {
                &ev.job.name[..28]
            } else {
                &ev.job.name
            };
            let run_id = if ev.run.run_id.len() > 8 {
                &ev.run.run_id[..8]
            } else {
                &ev.run.run_id
            };
            println!("{:<20} {:<10} {:<30} {:<8}", time, event_type, card, run_id);
        }
        println!("\n{} event(s) total", events.len());
    }
    Ok(())
}

pub fn cmd_events_check(root: &Path) -> anyhow::Result<()> {
    let events_path = root.join("events.jsonl");
    if !events_path.exists() {
        println!("WARN: no events.jsonl — lineage not enabled or never triggered");
        std::process::exit(1);
    }

    let content = fs::read_to_string(&events_path)?;
    let mut valid = 0usize;
    let mut parse_errors = 0usize;
    let mut cards: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut has_start = false;
    let mut has_terminal = false;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<jobcard_core::lineage::RunEvent>(line) {
            Ok(ev) => {
                valid += 1;
                cards.insert(ev.job.name.clone());
                match ev.event_type {
                    jobcard_core::lineage::EventType::Start => has_start = true,
                    jobcard_core::lineage::EventType::Complete
                    | jobcard_core::lineage::EventType::Fail
                    | jobcard_core::lineage::EventType::Abort => has_terminal = true,
                    _ => {}
                }
            }
            Err(_) => parse_errors += 1,
        }
    }

    println!(
        "{} events, {} cards, {} parse errors",
        valid,
        cards.len(),
        parse_errors
    );

    if valid == 0 {
        println!("WARN: events.jsonl exists but contains no valid events");
        std::process::exit(1);
    }
    if parse_errors > 0 {
        println!("WARN: {} lines failed to parse", parse_errors);
    }
    if !has_start {
        println!("WARN: no START events found");
    }
    if !has_terminal {
        println!("WARN: no terminal events (COMPLETE/FAIL/ABORT) found");
    }

    if parse_errors == 0 && has_start && has_terminal {
        println!("OK");
    } else {
        std::process::exit(1);
    }

    Ok(())
}
