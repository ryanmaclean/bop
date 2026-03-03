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

/// Validate events.jsonl integrity. Returns (valid, parse_errors, has_start, has_terminal, card_count).
/// Extracted for testability — the public cmd_events_check wraps this with process::exit calls.
pub(crate) fn check_events_file(content: &str) -> (usize, usize, bool, bool, usize) {
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
    (valid, parse_errors, has_start, has_terminal, cards.len())
}

pub fn cmd_events_check(root: &Path) -> anyhow::Result<()> {
    let events_path = root.join("events.jsonl");
    if !events_path.exists() {
        println!("WARN: no events.jsonl — lineage not enabled or never triggered");
        std::process::exit(1);
    }

    let content = fs::read_to_string(&events_path)?;
    let (valid, parse_errors, has_start, has_terminal, card_count) = check_events_file(&content);

    println!(
        "{} events, {} cards, {} parse errors",
        valid, card_count, parse_errors
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_run_event(event_type: &str, card_name: &str, run_id: &str) -> String {
        serde_json::json!({
            "eventType": event_type,
            "eventTime": "2026-03-02T00:00:00Z",
            "run": { "runId": run_id },
            "job": { "namespace": "bop", "name": card_name },
            "inputs": [],
            "outputs": [],
            "producer": "https://github.com/yourorg/bop",
            "schemaUrl": "https://openlineage.io/spec/2-0-2/OpenLineage.json#/$defs/RunEvent"
        })
        .to_string()
    }

    #[test]
    fn cmd_events_no_file_prints_message() {
        let td = tempdir().unwrap();
        cmd_events(td.path(), None, false, 50).unwrap();
    }

    #[test]
    fn cmd_events_filters_by_card_id() {
        let td = tempdir().unwrap();
        let mut content = String::new();
        content.push_str(&make_run_event("START", "card-alpha", "r1"));
        content.push('\n');
        content.push_str(&make_run_event("COMPLETE", "card-beta", "r2"));
        content.push('\n');
        fs::write(td.path().join("events.jsonl"), &content).unwrap();
        cmd_events(td.path(), Some("alpha"), false, 50).unwrap();
    }

    #[test]
    fn cmd_events_respects_limit() {
        let td = tempdir().unwrap();
        let mut content = String::new();
        for i in 0..10 {
            content.push_str(&make_run_event(
                "START",
                &format!("card-{}", i),
                &format!("r{}", i),
            ));
            content.push('\n');
        }
        fs::write(td.path().join("events.jsonl"), &content).unwrap();
        cmd_events(td.path(), None, false, 3).unwrap();
    }

    #[test]
    fn cmd_events_json_mode() {
        let td = tempdir().unwrap();
        let content = format!("{}\n", make_run_event("START", "test-card", "r1"));
        fs::write(td.path().join("events.jsonl"), &content).unwrap();
        cmd_events(td.path(), None, true, 50).unwrap();
    }

    #[test]
    fn check_reports_parse_errors() {
        let content = format!(
            "{}\nnot-valid-json\n",
            make_run_event("START", "card-a", "r1")
        );
        let (valid, parse_errors, _, _, _) = check_events_file(&content);
        assert_eq!(valid, 1);
        assert_eq!(parse_errors, 1);
    }

    #[test]
    fn check_detects_missing_start() {
        let content = format!("{}\n", make_run_event("COMPLETE", "card-a", "r1"));
        let (valid, parse_errors, has_start, has_terminal, _) = check_events_file(&content);
        assert_eq!(valid, 1);
        assert_eq!(parse_errors, 0);
        assert!(!has_start);
        assert!(has_terminal);
    }

    #[test]
    fn check_detects_missing_terminal() {
        let content = format!("{}\n", make_run_event("START", "card-a", "r1"));
        let (valid, _, has_start, has_terminal, _) = check_events_file(&content);
        assert_eq!(valid, 1);
        assert!(has_start);
        assert!(!has_terminal);
    }

    #[test]
    fn check_ok_for_valid_start_and_complete() {
        let content = format!(
            "{}\n{}\n",
            make_run_event("START", "card-a", "r1"),
            make_run_event("COMPLETE", "card-a", "r1"),
        );
        let (valid, parse_errors, has_start, has_terminal, card_count) =
            check_events_file(&content);
        assert_eq!(valid, 2);
        assert_eq!(parse_errors, 0);
        assert!(has_start);
        assert!(has_terminal);
        assert_eq!(card_count, 1);
    }

    #[test]
    fn check_empty_content() {
        let (valid, parse_errors, has_start, has_terminal, _) = check_events_file("");
        assert_eq!(valid, 0);
        assert_eq!(parse_errors, 0);
        assert!(!has_start);
        assert!(!has_terminal);
    }
}
