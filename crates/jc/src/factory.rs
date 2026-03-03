// ── factory (launchd lifecycle) ──────────────────────────────────────────────

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use crate::icons::{cmd_icons_install, cmd_icons_uninstall, ICONS_LABEL};

pub const FACTORY_LABELS: [(&str, &str); 2] = [
    ("sh.bop.dispatcher", "dispatcher"),
    ("sh.bop.merge-gate", "merge-gate"),
];

pub fn zellij_plugin_src(repo_root: &Path) -> PathBuf {
    repo_root.join("crates/jc-zellij-plugin/target/wasm32-wasip1/release/jc_zellij_plugin.wasm")
}

pub fn zellij_plugin_dest() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    Path::new(&home).join(".config/zellij/plugins/bop.wasm")
}

pub fn launchd_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set")).join("Library/LaunchAgents")
}

pub fn plist_path(label: &str) -> PathBuf {
    launchd_dir().join(format!("{}.plist", label))
}

pub fn generate_plist(label: &str, subcommand: &str, repo_root: &Path) -> String {
    let bop_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/bop"));
    let cards_dir = repo_root.join(".cards");
    let log_base = format!("/tmp/bop-{}", subcommand);

    // Extra args for dispatcher
    let mut extra_args = String::new();
    if subcommand == "dispatcher" {
        extra_args = r#"    <string>--adapter</string>
    <string>adapters/claude.zsh</string>
    <string>--max-workers</string>
    <string>3</string>
    <string>--poll-ms</string>
    <string>500</string>
    <string>--max-retries</string>
    <string>3</string>
    <string>--reap-ms</string>
    <string>1000</string>"#
            .to_string();
    }

    let args_block = if extra_args.is_empty() {
        format!(
            r#"    <string>{bin}</string>
    <string>{sub}</string>"#,
            bin = bop_bin.display(),
            sub = subcommand,
        )
    } else {
        format!(
            r#"    <string>{bin}</string>
    <string>{sub}</string>
{extra}"#,
            bin = bop_bin.display(),
            sub = subcommand,
            extra = extra_args,
        )
    };

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>

  <key>ProgramArguments</key>
  <array>
{args}
  </array>

  <key>WorkingDirectory</key>
  <string>{wd}</string>

  <key>EnvironmentVariables</key>
  <dict>
    <key>CARDS_DIR</key>
    <string>{cards}</string>
    <key>PATH</key>
    <string>/usr/local/bin:/usr/bin:/bin:{cargo_bin}</string>
    <key>RUST_LOG</key>
    <string>info</string>
  </dict>

  <key>KeepAlive</key>
  <true/>

  <key>RunAtLoad</key>
  <true/>

  <key>StandardOutPath</key>
  <string>{log_base}.log</string>

  <key>StandardErrorPath</key>
  <string>{log_base}.err</string>

  <key>HardResourceLimits</key>
  <dict>
    <key>NumberOfFiles</key>
    <integer>1024</integer>
  </dict>

  <key>SoftResourceLimits</key>
  <dict>
    <key>NumberOfFiles</key>
    <integer>512</integer>
  </dict>
</dict>
</plist>
"#,
        label = label,
        args = args_block,
        wd = repo_root.display(),
        cards = cards_dir.display(),
        cargo_bin = PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".cargo/bin")
            .display(),
        log_base = log_base,
    )
}

pub fn cmd_factory_install(cards_root: &Path) -> anyhow::Result<()> {
    // Resolve repo root (cards_root is .cards, parent is repo)
    let repo_root = fs::canonicalize(cards_root)
        .unwrap_or_else(|_| cards_root.to_path_buf())
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // Use cards_root's parent if cards_root ends with ".cards"
    let repo_root = if cards_root
        .file_name()
        .map(|f| f == ".cards")
        .unwrap_or(false)
    {
        cards_root.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        repo_root
    };
    let repo_root = fs::canonicalize(&repo_root).unwrap_or(repo_root);

    let la_dir = launchd_dir();
    fs::create_dir_all(&la_dir)?;

    for (label, subcmd) in &FACTORY_LABELS {
        let plist = generate_plist(label, subcmd, &repo_root);
        let dest = plist_path(label);
        fs::write(&dest, &plist)?;
        println!("✓ wrote {}", dest.display());
    }

    // Load both
    for (label, _) in &FACTORY_LABELS {
        let dest = plist_path(label);
        let out = StdCommand::new("launchctl")
            .args(["load", "-w"])
            .arg(&dest)
            .output()?;
        if out.status.success() {
            println!("✓ loaded {}", label);
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            eprintln!("⚠ load {}: {}", label, err.trim());
        }
    }

    // Icons watcher: default on, same lifecycle as factory
    if cfg!(target_os = "macos") {
        match cmd_icons_install(cards_root) {
            Ok(_) => {}
            Err(e) => eprintln!("⚠ icon watcher: {}", e),
        }
    }

    // Zellij plugin
    let wasm_src = zellij_plugin_src(&repo_root);
    if wasm_src.exists() {
        let dest = zellij_plugin_dest();
        if let Some(parent) = dest.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::copy(&wasm_src, &dest) {
            Ok(_) => println!("✓ zellij plugin installed: {}", dest.display()),
            Err(e) => eprintln!("  zellij plugin copy failed: {}", e),
        }
    } else {
        println!(
            "  (zellij plugin wasm not built — skipping)\n  build with: cargo build --manifest-path crates/jc-zellij-plugin/Cargo.toml --target wasm32-wasip1 --release"
        );
    }

    println!("\nFactory services installed. Run `bop factory status` to verify.");
    Ok(())
}

pub fn cmd_factory_start() -> anyhow::Result<()> {
    for (label, _) in &FACTORY_LABELS {
        let out = StdCommand::new("launchctl")
            .args(["start", label])
            .output()?;
        if out.status.success() {
            println!("✓ started {}", label);
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            eprintln!("⚠ start {}: {}", label, err.trim());
        }
    }
    Ok(())
}

pub fn cmd_factory_stop() -> anyhow::Result<()> {
    for (label, _) in &FACTORY_LABELS {
        let out = StdCommand::new("launchctl")
            .args(["stop", label])
            .output()?;
        if out.status.success() {
            println!("■ stopped {}", label);
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            eprintln!("⚠ stop {}: {}", label, err.trim());
        }
    }
    Ok(())
}

pub fn cmd_factory_status() -> anyhow::Result<()> {
    println!("── factory services ──");
    factory_status_one(ICONS_LABEL, "icons");
    for (label, subcmd) in &FACTORY_LABELS {
        let dest = plist_path(label);
        let installed = dest.exists();

        // Check if loaded via launchctl list
        let out = StdCommand::new("launchctl")
            .args(["list", label])
            .output()?;
        let loaded = out.status.success();

        let pid = if loaded {
            // Parse PID from launchctl list output (first field of matching line)
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.lines().find(|l| l.contains("PID")).and_then(|_| {
                // launchctl list <label> outputs key-value pairs
                stdout.lines().find_map(|l| {
                    let l = l.trim();
                    if l.starts_with("\"PID\"") || l.starts_with("PID") {
                        l.split('=')
                            .nth(1)
                            .or_else(|| l.split_whitespace().nth(1))
                            .and_then(|v| v.trim().trim_matches(';').parse::<u32>().ok())
                    } else {
                        None
                    }
                })
            })
        } else {
            None
        };

        let status_str = match (installed, loaded, pid) {
            (true, true, Some(p)) => format!("● running (pid {})", p),
            (true, true, None) => "● loaded (waiting)".to_string(),
            (true, false, _) => "○ installed (not loaded)".to_string(),
            (false, _, _) => "□ not installed".to_string(),
        };
        println!("  {} {}: {}", subcmd, label, status_str);

        // Show log paths
        let log_path = format!("/tmp/bop-{}.log", subcmd);
        let err_path = format!("/tmp/bop-{}.err", subcmd);
        if Path::new(&log_path).exists() {
            println!("    stdout: {}", log_path);
        }
        if Path::new(&err_path).exists() {
            println!("    stderr: {}", err_path);
        }
    }
    Ok(())
}

pub fn cmd_factory_uninstall() -> anyhow::Result<()> {
    // Icons watcher travels with factory
    let _ = cmd_icons_uninstall();

    // Zellij plugin
    let zj_dest = zellij_plugin_dest();
    if zj_dest.exists() {
        let _ = fs::remove_file(&zj_dest);
        println!("✓ removed zellij plugin: {}", zj_dest.display());
    } else {
        println!("  (zellij plugin not installed)");
    }

    for (label, _) in &FACTORY_LABELS {
        let dest = plist_path(label);

        // Unload first (ignore errors if not loaded)
        let _ = StdCommand::new("launchctl")
            .args(["unload", "-w"])
            .arg(&dest)
            .output();

        if dest.exists() {
            fs::remove_file(&dest)?;
            println!("✓ removed {}", dest.display());
        } else {
            println!("  (not installed: {})", label);
        }
    }
    println!("\nFactory services uninstalled.");
    Ok(())
}

pub fn factory_status_one(label: &str, name: &str) {
    let dest = plist_path(label);
    let installed = dest.exists();
    let out = StdCommand::new("launchctl")
        .args(["list", label])
        .output()
        .ok();
    let loaded = out.as_ref().map(|o| o.status.success()).unwrap_or(false);
    let status_str = match (installed, loaded) {
        (true, true) => "● active",
        (true, false) => "○ installed (not loaded)",
        (false, _) => "□ not installed",
    };
    println!("  {} {}: {}", name, label, status_str);
}
