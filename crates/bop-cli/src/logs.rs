use anyhow::Context;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

use crate::paths;

pub async fn cmd_logs(root: &Path, id: &str, follow: bool) -> anyhow::Result<()> {
    use std::io::IsTerminal;
    let card = paths::find_card(root, id).with_context(|| format!("card not found: {}", id))?;
    let stdout_log = card.join("logs").join("stdout.log");
    let stderr_log = card.join("logs").join("stderr.log");
    let is_tty = std::io::stdout().is_terminal();

    if !follow {
        // Print all existing content once
        print_log_section("stdout", &stdout_log, is_tty)?;
        print_log_section("stderr", &stderr_log, is_tty)?;
        return Ok(());
    }

    // --follow: open both files and stream new bytes as they arrive
    let mut stdout_file = fs::File::open(&stdout_log)
        .unwrap_or_else(|_| fs::File::open("/dev/null").expect("open /dev/null"));
    let mut stderr_file = fs::File::open(&stderr_log)
        .unwrap_or_else(|_| fs::File::open("/dev/null").expect("open /dev/null"));

    // Drain any existing content first
    let mut buf = Vec::new();
    stdout_file.read_to_end(&mut buf)?;
    if !buf.is_empty() {
        print!("{}", colorize_chunk(&buf, is_tty));
    }
    let mut stdout_pos = stdout_file.stream_position()?;
    buf.clear();

    stderr_file.read_to_end(&mut buf)?;
    if !buf.is_empty() {
        eprint!("{}", colorize_chunk(&buf, is_tty));
    }
    let mut stderr_pos = stderr_file.stream_position()?;

    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Re-open if file was rotated/created after we started
        if !stdout_log.exists() {
            if let Ok(f) = fs::File::open(&stdout_log) {
                stdout_file = f;
                stdout_pos = 0;
            }
        }
        if !stderr_log.exists() {
            if let Ok(f) = fs::File::open(&stderr_log) {
                stderr_file = f;
                stderr_pos = 0;
            }
        }

        // Read any new bytes from stdout
        stdout_file.seek(SeekFrom::Start(stdout_pos))?;
        buf.clear();
        stdout_file.read_to_end(&mut buf)?;
        if !buf.is_empty() {
            print!("{}", colorize_chunk(&buf, is_tty));
            std::io::stdout().flush()?;
            stdout_pos += buf.len() as u64;
        }

        // Read any new bytes from stderr
        stderr_file.seek(SeekFrom::Start(stderr_pos))?;
        buf.clear();
        stderr_file.read_to_end(&mut buf)?;
        if !buf.is_empty() {
            eprint!("{}", colorize_chunk(&buf, is_tty));
            std::io::stderr().flush()?;
            stderr_pos += buf.len() as u64;
        }

        // Stop following once the card leaves running/
        let still_running = paths::find_card_in_state(root, id, "running");
        if !still_running {
            break;
        }
    }

    Ok(())
}

fn colorize_log_line(line: &str) -> String {
    const R: &str = "\x1b[0m";
    if line.contains("ERROR") || line.contains("error:") || line.contains("FAILED") {
        return format!("\x1b[1;31m{}{}", line, R);
    }
    if line.contains("WARN") || line.contains("warning:") {
        return format!("\x1b[33m{}{}", line, R);
    }
    if line.contains("INFO") {
        return format!("\x1b[36m{}{}", line, R);
    }
    if line.contains("DEBUG") || line.contains("TRACE") {
        return format!("\x1b[2m{}{}", line, R);
    }
    if line.contains("→ merged") || line.contains("-> merged") {
        return format!("\x1b[1;35m{}{}", line, R);
    }
    if line.contains("→ done") || line.contains("-> done") {
        return format!("\x1b[1;32m{}{}", line, R);
    }
    if line.contains("→ failed") || line.contains("-> failed") {
        return format!("\x1b[1;31m{}{}", line, R);
    }
    if line.contains("→ running") || line.contains("-> running") {
        return format!("\x1b[1;33m{}{}", line, R);
    }
    line.to_string()
}

fn colorize_chunk(bytes: &[u8], is_tty: bool) -> String {
    let text = String::from_utf8_lossy(bytes);
    if !is_tty {
        return text.into_owned();
    }
    text.lines()
        .map(colorize_log_line)
        .collect::<Vec<_>>()
        .join("\n")
        + if text.ends_with('\n') { "\n" } else { "" }
}

fn print_log_section(label: &str, path: &Path, is_tty: bool) -> anyhow::Result<()> {
    if !path.exists() {
        println!("=== {} (no file) ===", label);
        return Ok(());
    }
    let content = fs::read_to_string(path)?;
    if is_tty {
        println!("\x1b[1m=== {} ===\x1b[0m", label);
    } else {
        println!("=== {} ===", label);
    }
    if is_tty {
        for line in content.lines() {
            println!("{}", colorize_log_line(line));
        }
        if !content.ends_with('\n') && !content.is_empty() {
            // already printed trailing newline via println
        }
    } else {
        print!("{}", content);
        if !content.ends_with('\n') && !content.is_empty() {
            println!();
        }
    }
    Ok(())
}
