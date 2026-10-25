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
            if let Some(path) = env::var_os("BOP_TIMEOUT_TREE_TARGET_DIR") {
                fs::create_dir_all(&path)?;
                fs::write(
                    std::path::Path::new(&path).join("owned-fixture"),
                    b"native work began",
                )?;
            }
            let mut command = Command::new(env::current_exe()?);
            #[cfg(unix)]
            if env::var_os("BOP_TIMEOUT_TREE_SEPARATE_SESSION").is_some() {
                use std::os::unix::process::CommandExt;
                // Only the async-signal-safe native setsid call runs after fork.
                unsafe {
                    command.pre_exec(|| {
                        if libc::setsid() == -1 {
                            Err(std::io::Error::last_os_error())
                        } else {
                            Ok(())
                        }
                    });
                }
            }
            let mut grandchild = command
                .arg("grandchild")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let _ = grandchild.wait()?;
        }
        Some("grandchild" | "sentinel") => {
            let pid_file = required_path("BOP_TIMEOUT_TREE_PID_FILE")?;
            let beat_file = required_path("BOP_TIMEOUT_TREE_BEAT_FILE")?;
            fs::write(pid_file, std::process::id().to_string())?;
            // Finite lifetime bounds any orphan if the regression reappears.
            let ticks = if args.first().map(String::as_str) == Some("sentinel") {
                1200
            } else {
                150
            };
            for _ in 0..ticks {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&beat_file)?
                    .write_all(b"x")?;
                thread::sleep(Duration::from_millis(100));
            }
        }
        _ => return Err("unsupported fixture invocation".into()),
    }
    Ok(())
}
