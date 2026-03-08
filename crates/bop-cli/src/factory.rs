// ── factory (launchd lifecycle + systemd) ────────────────────────────────────

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use crate::icons::{cmd_icons_install, cmd_icons_uninstall, ICONS_LABEL};
use crate::pool;

pub const FACTORY_LABELS: [(&str, &str); 2] = [
    ("sh.bop.dispatcher", "dispatcher"),
    ("sh.bop.merge-gate", "merge-gate"),
];
const DISPATCHER_ADAPTER_FALLBACK: &str = "adapters/claude.nu";

fn is_linux() -> bool {
    cfg!(target_os = "linux")
}

fn is_macos() -> bool {
    cfg!(target_os = "macos")
}

fn launchd_user_domain() -> anyhow::Result<String> {
    if let Ok(uid) = std::env::var("UID") {
        let uid = uid.trim();
        if !uid.is_empty() {
            return Ok(format!("gui/{uid}"));
        }
    }

    let out = StdCommand::new("id").arg("-u").output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("failed to resolve UID via `id -u`: {}", err.trim());
    }

    let uid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if uid.is_empty() {
        anyhow::bail!("failed to resolve UID: empty `id -u` output");
    }
    Ok(format!("gui/{uid}"))
}

pub fn zellij_plugin_src(repo_root: &Path) -> PathBuf {
    repo_root.join("crates/bop-zellij-plugin/target/wasm32-wasip1/release/bop_zellij_plugin.wasm")
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

pub fn systemd_user_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set")).join(".config/systemd/user")
}

pub fn systemd_service_path(label: &str) -> PathBuf {
    systemd_user_dir().join(format!("{}.service", label))
}

pub fn systemd_path_path(label: &str) -> PathBuf {
    systemd_user_dir().join(format!("{}.path", label))
}

pub fn generate_plist(label: &str, subcommand: &str, repo_root: &Path) -> String {
    let bop_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/bop"));
    let cards_dir = repo_root.join(".cards");
    let log_base = format!("/tmp/bop-{}", subcommand);

    // Build WatchPaths: base dir + team dirs
    let watch_subdir = if subcommand == "dispatcher" {
        "pending"
    } else {
        "done"
    };

    let mut watch_paths = vec![cards_dir.join(watch_subdir)];

    // Discover team-* directories
    if let Ok(entries) = fs::read_dir(&cards_dir) {
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                if name.starts_with("team-") {
                    let team_watch = cards_dir.join(&name).join(watch_subdir);
                    if team_watch.exists() {
                        watch_paths.push(team_watch);
                    }
                }
            }
        }
    }

    // Format WatchPaths as XML array items
    let watch_paths_xml = watch_paths
        .iter()
        .map(|p| format!("    <string>{}</string>", p.display()))
        .collect::<Vec<_>>()
        .join("\n");

    // Extra args for dispatcher.
    // --adapter is kept as an explicit fallback; per-card routing is resolved by dispatcher.
    let mut extra_args = String::new();
    if subcommand == "dispatcher" {
        extra_args = format!(
            r#"    <string>--vcs-engine</string>
    <string>jj</string>
    <string>--adapter</string>
    <string>{}</string>
    <string>--max-workers</string>
    <string>3</string>
    <string>--once</string>
    <string>--max-retries</string>
    <string>3</string>"#,
            DISPATCHER_ADAPTER_FALLBACK
        );
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

  <key>WatchPaths</key>
  <array>
{watch_paths}
  </array>

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
        watch_paths = watch_paths_xml,
        args = args_block,
        wd = repo_root.display(),
        cards = cards_dir.display(),
        cargo_bin = PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".cargo/bin")
            .display(),
        log_base = log_base,
    )
}

pub fn generate_systemd_service(_label: &str, subcommand: &str, repo_root: &Path) -> String {
    let bop_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/bop"));
    let cards_dir = repo_root.join(".cards");
    let log_base = format!("/tmp/bop-{}", subcommand);
    let cargo_bin = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cargo/bin");

    // Build command line with args
    let mut cmd_args = vec![bop_bin.display().to_string(), subcommand.to_string()];

    if subcommand == "dispatcher" {
        cmd_args.extend([
            "--vcs-engine".to_string(),
            "jj".to_string(),
            "--adapter".to_string(),
            DISPATCHER_ADAPTER_FALLBACK.to_string(),
            "--max-workers".to_string(),
            "3".to_string(),
            "--once".to_string(),
            "--max-retries".to_string(),
            "3".to_string(),
        ]);
    }

    let exec_start = cmd_args.join(" ");
    let log_file = format!("{log_base}.log");
    let err_file = format!("{log_base}.err");

    format!(
        r#"[Unit]
Description=bop {} service
After=network.target

[Service]
Type=oneshot
WorkingDirectory={}
Environment="CARDS_DIR={}"
Environment="PATH=/usr/local/bin:/usr/bin:/bin:{}"
Environment="RUST_LOG=info"
ExecStart={}
StandardOutput=append:{}
StandardError=append:{}
LimitNOFILE=1024

[Install]
WantedBy=default.target
"#,
        subcommand,
        repo_root.display(),
        cards_dir.display(),
        cargo_bin.display(),
        exec_start,
        log_file,
        err_file,
    )
}

pub fn generate_systemd_path(_label: &str, subcommand: &str, repo_root: &Path) -> String {
    let cards_dir = repo_root.join(".cards");

    // Build watch paths: base dir + team dirs
    let watch_subdir = if subcommand == "dispatcher" {
        "pending"
    } else {
        "done"
    };

    let mut watch_paths = vec![cards_dir.join(watch_subdir)];

    // Discover team-* directories
    if let Ok(entries) = fs::read_dir(&cards_dir) {
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                if name.starts_with("team-") {
                    let team_watch = cards_dir.join(&name).join(watch_subdir);
                    if team_watch.exists() {
                        watch_paths.push(team_watch);
                    }
                }
            }
        }
    }

    // Format PathChanged directives
    let path_changed = watch_paths
        .iter()
        .map(|p| format!("PathChanged={}", p.display()))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"[Unit]
Description=Watch {} directory for bop {}

[Path]
{}

[Install]
WantedBy=default.target
"#,
        watch_subdir, subcommand, path_changed
    )
}

fn resolve_repo_root(cards_root: &Path) -> PathBuf {
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
    fs::canonicalize(&repo_root).unwrap_or(repo_root)
}

fn install_zellij_plugin(repo_root: &Path) {
    let wasm_src = zellij_plugin_src(repo_root);
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
            "  (zellij plugin wasm not built — skipping)\n  build with: cargo build --manifest-path crates/bop-zellij-plugin/Cargo.toml --target wasm32-wasip1 --release"
        );
    }
}

fn cmd_factory_install_macos(repo_root: &Path, cards_root: &Path) -> anyhow::Result<()> {
    let la_dir = launchd_dir();
    fs::create_dir_all(&la_dir)?;
    let domain = launchd_user_domain()?;

    for (label, subcmd) in &FACTORY_LABELS {
        let plist = generate_plist(label, subcmd, repo_root);
        let dest = plist_path(label);
        fs::write(&dest, &plist)?;
        println!("✓ wrote {}", dest.display());

        // Replace any existing loaded definition before bootstrapping the new plist.
        let _ = StdCommand::new("launchctl")
            .args(["bootout", &domain])
            .arg(&dest)
            .output();

        let out = StdCommand::new("launchctl")
            .args(["bootstrap", &domain])
            .arg(&dest)
            .output()?;
        if out.status.success() {
            println!("✓ bootstrapped {}", label);
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            eprintln!("⚠ bootstrap {}: {}", label, err.trim());
        }
    }

    // Icons watcher: default on, same lifecycle as factory
    match cmd_icons_install(cards_root) {
        Ok(_) => {}
        Err(e) => eprintln!("⚠ icon watcher: {}", e),
    }

    Ok(())
}

fn cmd_factory_install_linux(repo_root: &Path) -> anyhow::Result<()> {
    let systemd_dir = systemd_user_dir();
    fs::create_dir_all(&systemd_dir)?;

    for (label, subcmd) in &FACTORY_LABELS {
        // Write service file
        let service = generate_systemd_service(label, subcmd, repo_root);
        let service_path = systemd_service_path(label);
        fs::write(&service_path, &service)?;
        println!("✓ wrote {}", service_path.display());

        // Write path unit
        let path_unit = generate_systemd_path(label, subcmd, repo_root);
        let path_path = systemd_path_path(label);
        fs::write(&path_path, &path_unit)?;
        println!("✓ wrote {}", path_path.display());
    }

    // Reload daemon
    let reload_out = StdCommand::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()?;
    if !reload_out.status.success() {
        let err = String::from_utf8_lossy(&reload_out.stderr);
        anyhow::bail!("systemctl --user daemon-reload failed: {}", err.trim());
    }

    // Enable and start both service + path units.
    for (label, _) in &FACTORY_LABELS {
        let service_unit = format!("{}.service", label);
        let path_unit = format!("{}.path", label);

        for unit in [&service_unit, &path_unit] {
            let enable_out = StdCommand::new("systemctl")
                .args(["--user", "enable", unit])
                .output()?;
            if enable_out.status.success() {
                println!("✓ enabled {}", unit);
            } else {
                let err = String::from_utf8_lossy(&enable_out.stderr);
                anyhow::bail!("systemctl --user enable {} failed: {}", unit, err.trim());
            }

            let start_out = StdCommand::new("systemctl")
                .args(["--user", "start", unit])
                .output()?;
            if start_out.status.success() {
                println!("✓ started {}", unit);
            } else {
                let err = String::from_utf8_lossy(&start_out.stderr);
                anyhow::bail!("systemctl --user start {} failed: {}", unit, err.trim());
            }
        }
    }

    Ok(())
}

pub fn cmd_factory_install(cards_root: &Path) -> anyhow::Result<()> {
    let repo_root = resolve_repo_root(cards_root);

    if is_macos() {
        cmd_factory_install_macos(&repo_root, cards_root)?;
    } else if is_linux() {
        cmd_factory_install_linux(&repo_root)?;
    } else {
        anyhow::bail!("Unsupported OS: factory install only works on macOS and Linux");
    }

    install_zellij_plugin(&repo_root);

    println!("\nFactory services installed. Run `bop factory status` to verify.");
    Ok(())
}

pub fn cmd_factory_start() -> anyhow::Result<()> {
    if is_macos() {
        let domain = launchd_user_domain()?;
        for (label, _) in &FACTORY_LABELS {
            let service_target = format!("{}/{}", domain, label);
            let out = StdCommand::new("launchctl")
                .args(["kickstart", "-k", &service_target])
                .output()?;
            if out.status.success() {
                println!("✓ started {}", label);
            } else {
                let err = String::from_utf8_lossy(&out.stderr);
                eprintln!("⚠ start {}: {}", label, err.trim());
            }
        }
    } else if is_linux() {
        for (label, _) in &FACTORY_LABELS {
            let path_unit = format!("{}.path", label);
            let out = StdCommand::new("systemctl")
                .args(["--user", "start", &path_unit])
                .output()?;
            if out.status.success() {
                println!("✓ started {}", path_unit);
            } else {
                let err = String::from_utf8_lossy(&out.stderr);
                eprintln!("⚠ start {}: {}", path_unit, err.trim());
            }
        }
    } else {
        anyhow::bail!("Unsupported OS");
    }
    Ok(())
}

pub fn cmd_factory_stop() -> anyhow::Result<()> {
    if is_macos() {
        let domain = launchd_user_domain()?;
        for (label, _) in &FACTORY_LABELS {
            let service_target = format!("{}/{}", domain, label);
            let out = StdCommand::new("launchctl")
                .args(["stop", &service_target])
                .output()?;
            if out.status.success() {
                println!("■ stopped {}", label);
            } else {
                let err = String::from_utf8_lossy(&out.stderr);
                eprintln!("⚠ stop {}: {}", label, err.trim());
            }
        }
    } else if is_linux() {
        for (label, _) in &FACTORY_LABELS {
            let path_unit = format!("{}.path", label);
            let out = StdCommand::new("systemctl")
                .args(["--user", "stop", &path_unit])
                .output()?;
            if out.status.success() {
                println!("■ stopped {}", path_unit);
            } else {
                let err = String::from_utf8_lossy(&out.stderr);
                eprintln!("⚠ stop {}: {}", path_unit, err.trim());
            }
        }
    } else {
        anyhow::bail!("Unsupported OS");
    }
    Ok(())
}

pub fn cmd_factory_status() -> anyhow::Result<()> {
    println!("── factory services ──");

    if is_macos() {
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
    } else if is_linux() {
        for (label, subcmd) in &FACTORY_LABELS {
            let service_path = systemd_service_path(label);
            let path_path = systemd_path_path(label);
            let installed = service_path.exists() && path_path.exists();

            let path_unit = format!("{}.path", label);

            // Check if active via systemctl
            let out = StdCommand::new("systemctl")
                .args(["--user", "is-active", &path_unit])
                .output()?;
            let active = out.status.success();

            // Get main PID from systemctl show (for the service unit)
            let service_unit = format!("{}.service", label);
            let show_out = StdCommand::new("systemctl")
                .args(["--user", "show", "-p", "MainPID", &service_unit])
                .output()
                .ok();

            let pid = show_out.and_then(|o| {
                let stdout = String::from_utf8_lossy(&o.stdout);
                stdout
                    .trim()
                    .strip_prefix("MainPID=")
                    .and_then(|p| p.parse::<u32>().ok())
                    .filter(|&p| p != 0)
            });

            let status_str = match (installed, active, pid) {
                (true, true, Some(p)) => format!("● active (pid {})", p),
                (true, true, None) => "● active (waiting)".to_string(),
                (true, false, _) => "○ installed (inactive)".to_string(),
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
    } else {
        anyhow::bail!("Unsupported OS");
    }

    Ok(())
}

pub fn cmd_factory_uninstall() -> anyhow::Result<()> {
    // Icons watcher travels with factory (macOS only)
    if is_macos() {
        let _ = cmd_icons_uninstall();
    }

    // Zellij plugin
    let zj_dest = zellij_plugin_dest();
    if zj_dest.exists() {
        let _ = fs::remove_file(&zj_dest);
        println!("✓ removed zellij plugin: {}", zj_dest.display());
    } else {
        println!("  (zellij plugin not installed)");
    }

    if is_macos() {
        let domain = launchd_user_domain().ok();
        for (label, _) in &FACTORY_LABELS {
            let dest = plist_path(label);

            // Remove from launchd first (ignore errors if not loaded)
            if let Some(domain) = &domain {
                let _ = StdCommand::new("launchctl")
                    .args(["bootout", domain])
                    .arg(&dest)
                    .output();
            } else {
                let _ = StdCommand::new("launchctl")
                    .args(["unload", "-w"])
                    .arg(&dest)
                    .output();
            }

            if dest.exists() {
                fs::remove_file(&dest)?;
                println!("✓ removed {}", dest.display());
            } else {
                println!("  (not installed: {})", label);
            }
        }
    } else if is_linux() {
        for (label, _) in &FACTORY_LABELS {
            let service_unit = format!("{}.service", label);
            let path_unit = format!("{}.path", label);
            let service_path = systemd_service_path(label);
            let path_path = systemd_path_path(label);

            // Stop and disable both units.
            for unit in [&path_unit, &service_unit] {
                let _ = StdCommand::new("systemctl")
                    .args(["--user", "stop", unit])
                    .output();
                let _ = StdCommand::new("systemctl")
                    .args(["--user", "disable", unit])
                    .output();
            }

            // Remove files
            if service_path.exists() {
                fs::remove_file(&service_path)?;
                println!("✓ removed {}", service_path.display());
            }
            if path_path.exists() {
                fs::remove_file(&path_path)?;
                println!("✓ removed {}", path_path.display());
            }

            if !service_path.exists() && !path_path.exists() {
                println!("  (not installed: {})", label);
            }
        }

        // Reload daemon
        let _ = StdCommand::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();
    } else {
        anyhow::bail!("Unsupported OS");
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

// ── factory pool (QEMU prewarm) ──────────────────────────────────────────────

pub fn cmd_factory_pool_size(cards_root: &Path, size: usize) -> anyhow::Result<()> {
    pool::cmd_pool_set_size(cards_root, size)
}

pub fn cmd_factory_pool_status(cards_root: &Path) -> anyhow::Result<()> {
    pool::cmd_pool_status(cards_root)
}

pub fn cmd_factory_pool_stop(cards_root: &Path) -> anyhow::Result<()> {
    pool::cmd_pool_stop(cards_root)
}

pub fn cmd_factory_pool_monitor(cards_root: &Path) -> anyhow::Result<()> {
    pool::cmd_pool_monitor(cards_root)
}

pub fn cmd_factory_pool_lease(
    cards_root: &Path,
    card_id: &str,
    timeout_s: u64,
) -> anyhow::Result<()> {
    pool::cmd_pool_lease(cards_root, card_id, timeout_s)
}

pub fn cmd_factory_pool_release(
    cards_root: &Path,
    slot: usize,
    card_id: Option<&str>,
    exit_code: i32,
) -> anyhow::Result<()> {
    pool::cmd_pool_release(cards_root, slot, card_id, exit_code)
}
