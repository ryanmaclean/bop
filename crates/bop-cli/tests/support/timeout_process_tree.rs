//! Original, finite-lived subprocess fixture for the opt-in BOP timeout test.
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

fn required_path(name: &str) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    Ok(env::var_os(name).ok_or("missing fixture path")?.into())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") if args.iter().any(|arg| arg == "--help") => {
            println!("--prompt-stdin");
        }
        Some("run") => {
            let prompt_path = required_path("BOP_TIMEOUT_TREE_PROMPT_FILE")?;
            let mut prompt = Vec::new();
            std::io::stdin().read_to_end(&mut prompt)?;
            fs::write(prompt_path, &prompt)?;
            let mut grandchild = Command::new(env::current_exe()?)
                .arg("grandchild")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let _ = grandchild.wait()?;
        }
        Some("grandchild") => {
            let pid_file = required_path("BOP_TIMEOUT_TREE_PID_FILE")?;
            let beat_file = required_path("BOP_TIMEOUT_TREE_BEAT_FILE")?;
            fs::write(pid_file, std::process::id().to_string())?;
            // Finite lifetime bounds any orphan if the regression reappears.
            for _ in 0..150 {
                OpenOptions::new().create(true).append(true).open(&beat_file)?
                    .write_all(b"x")?;
                thread::sleep(Duration::from_millis(100));
            }
        }
        _ => return Err("unsupported fixture invocation".into()),
    }
    Ok(())
}
