use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};

const SEP: &str = "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━";

#[derive(Default)]
struct Tally {
    errors: usize,
    warnings: usize,
}

fn print_ok(message: impl AsRef<str>) {
    println!("  ✓ {}", message.as_ref());
}

fn print_warn(message: impl AsRef<str>, tally: &mut Tally) {
    println!("  ⚠ {}", message.as_ref());
    tally.warnings += 1;
}

fn print_err(message: impl AsRef<str>, tally: &mut Tally) {
    println!("  ✗ {}", message.as_ref());
    tally.errors += 1;
}

fn command_candidates(name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            out.push(dir.join(name));
        }
    }

    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        out.push(Path::new(&home).join(".local/bin").join(name));
        out.push(Path::new(&home).join(".cargo/bin").join(name));
    }
    out.push(PathBuf::from("/opt/homebrew/bin").join(name));

    out
}

fn command_available(name: &str) -> bool {
    command_candidates(name).into_iter().any(|candidate| {
        StdCommand::new(&candidate)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

fn command_version(name: &str) -> Option<String> {
    for candidate in command_candidates(name) {
        let output = match StdCommand::new(&candidate).arg("--version").output() {
            Ok(output) => output,
            Err(_) => continue,
        };
        if !output.status.success() {
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(line) = stdout.lines().next() {
            if !line.trim().is_empty() {
                if let Some(version) = extract_version(line) {
                    return Some(version);
                }
                return Some(line.trim().to_string());
            }
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        if let Some(line) = stderr.lines().next() {
            if !line.trim().is_empty() {
                if let Some(version) = extract_version(line) {
                    return Some(version);
                }
                return Some(line.trim().to_string());
            }
        }
    }
    None
}

fn extract_version(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|token| {
        let cleaned = token
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-')
            .trim_start_matches('v');
        if cleaned.chars().any(|c| c.is_ascii_digit()) && cleaned.contains('.') {
            Some(cleaned.to_string())
        } else {
            None
        }
    })
}

fn parse_semver_triplet(raw: &str) -> Option<(u64, u64, u64)> {
    let core = raw
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find(|part| part.contains('.'))?;
    let mut it = core.split('.');
    let major = it.next()?.parse::<u64>().ok()?;
    let minor = it.next().unwrap_or("0").parse::<u64>().ok()?;
    let patch = it.next().unwrap_or("0").parse::<u64>().ok()?;
    Some((major, minor, patch))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|m| (m.permissions().mode() & 0o111) != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        path.exists()
    }
}

fn check_keychain_credential(service: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        StdCommand::new("security")
            .args(["find-generic-password", "-s", service])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = service;
        false
    }
}

fn check_environment(tally: &mut Tally) -> bool {
    println!("  Environment");

    let mut nu_available = false;
    match command_version("nu") {
        Some(version) => {
            nu_available = true;
            let min = (0, 100, 0);
            match parse_semver_triplet(&version) {
                Some(found) if found >= min => {
                    print_ok(format!("nu {version}"));
                }
                Some(found) => {
                    print_err(
                        format!(
                            "nu {version} — requires >= 0.100.0 (found {}.{}.{})",
                            found.0, found.1, found.2
                        ),
                        tally,
                    );
                }
                None => {
                    print_err(
                        format!("nu {version} — could not parse version, requires >= 0.100.0"),
                        tally,
                    );
                }
            }
        }
        None => {
            print_err("nu — not found", tally);
        }
    }

    for cmd in ["jj", "git", "codex", "zellij", "cargo"] {
        match command_version(cmd) {
            Some(version) => print_ok(format!("{cmd} {version}")),
            None => print_err(format!("{cmd} — not found"), tally),
        }
    }

    println!();
    nu_available
}

fn check_filesystem(cards_root: &Path, fix: bool, tally: &mut Tally) -> Option<Vec<String>> {
    println!("  Filesystem");

    if cards_root.is_dir() {
        print_ok(format!("{} exists", cards_root.display()));
    } else if fix {
        match fs::create_dir_all(cards_root) {
            Ok(_) => print_ok(format!(
                "{} exists (created by --fix)",
                cards_root.display()
            )),
            Err(e) => print_err(
                format!(
                    "{} missing and could not be created: {}",
                    cards_root.display(),
                    e
                ),
                tally,
            ),
        }
    } else {
        print_err(
            format!("{} missing (run: bop init)", cards_root.display()),
            tally,
        );
    }

    let states = ["pending", "running", "done", "merged", "failed"];
    let mut present = 0usize;
    let mut created = 0usize;
    for state in states {
        let p = cards_root.join(state);
        if p.is_dir() {
            present += 1;
            continue;
        }
        if fix && fs::create_dir_all(&p).is_ok() {
            present += 1;
            created += 1;
        }
    }

    if present == 5 {
        if created > 0 {
            print_ok(format!(
                "state directories (5/5, created {} by --fix)",
                created
            ));
        } else {
            print_ok("state directories (5/5)");
        }
    } else {
        print_err(format!("state directories ({present}/5)"), tally);
    }

    let locks = cards_root.join(".locks");
    if locks.is_dir() {
        print_ok(format!("{} exists", locks.display()));
    } else {
        print_err(format!("{} missing", locks.display()), tally);
    }

    let providers_path = cards_root.join("providers.json");
    let provider_names = if providers_path.exists() {
        match fs::read_to_string(&providers_path) {
            Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(json) => {
                    print_ok(format!("{} valid JSON", providers_path.display()));
                    json.get("providers")
                        .and_then(|v| v.as_object())
                        .map(|m| m.keys().cloned().collect::<Vec<_>>())
                }
                Err(e) => {
                    print_err(
                        format!("{} invalid JSON: {}", providers_path.display(), e),
                        tally,
                    );
                    None
                }
            },
            Err(e) => {
                print_err(
                    format!("{} unreadable: {}", providers_path.display(), e),
                    tally,
                );
                None
            }
        }
    } else if fix {
        match crate::providers::seed_providers(cards_root) {
            Ok(_) => {
                print_ok(format!(
                    "{} created by --fix (default providers)",
                    providers_path.display()
                ));
                match fs::read_to_string(&providers_path)
                    .ok()
                    .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                    .and_then(|v| {
                        v.get("providers")
                            .and_then(|m| m.as_object())
                            .map(|m| m.keys().cloned().collect::<Vec<_>>())
                    }) {
                    Some(names) => Some(names),
                    None => {
                        print_warn(
                            format!(
                                "{} created but provider list could not be parsed",
                                providers_path.display()
                            ),
                            tally,
                        );
                        None
                    }
                }
            }
            Err(e) => {
                print_err(
                    format!(
                        "{} missing and could not be created: {}",
                        providers_path.display(),
                        e
                    ),
                    tally,
                );
                None
            }
        }
    } else {
        print_err(
            format!("{} missing (run: bop init)", providers_path.display()),
            tally,
        );
        None
    };

    let implement_template = cards_root.join("templates").join("implement.bop");
    if implement_template.is_dir() {
        print_ok(format!("{} exists", implement_template.display()));
    } else {
        print_err(format!("{} missing", implement_template.display()), tally);
    }

    println!();
    provider_names
}

fn check_adapters(cards_root: &Path, nu_available: bool, tally: &mut Tally) {
    println!("  Adapters");

    let repo_root = cards_root.parent().unwrap_or(cards_root);
    let adapters_dir = repo_root.join("adapters");
    let entries = match fs::read_dir(&adapters_dir) {
        Ok(entries) => entries,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                print_warn(
                    format!(
                        "{} not found (adapter checks skipped)",
                        adapters_dir.display()
                    ),
                    tally,
                );
            } else {
                print_err(
                    format!("{} unreadable: {}", adapters_dir.display(), e),
                    tally,
                );
            }
            println!();
            return;
        }
    };

    let mut adapter_files = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("nu"))
        .collect::<Vec<_>>();
    adapter_files.sort();

    if adapter_files.is_empty() {
        print_err("no adapters/*.nu found", tally);
    }

    for adapter in &adapter_files {
        let name = adapter
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>");
        if is_executable(adapter) {
            print_ok(format!("{} executable", name));
        } else {
            print_err(format!("{} not executable", name), tally);
        }
    }

    let mock = adapters_dir.join("mock.nu");
    if !mock.exists() {
        print_err(format!("{} missing", mock.display()), tally);
    } else if !nu_available {
        print_warn("mock.nu --test skipped (nu is not available)", tally);
    } else {
        match StdCommand::new("nu").arg(&mock).arg("--test").output() {
            Ok(out) if out.status.success() => {
                print_ok("mock.nu --test passed");
            }
            Ok(out) => {
                print_err(
                    format!("mock.nu --test failed (exit: {:?})", out.status.code()),
                    tally,
                );
            }
            Err(e) => {
                print_err(format!("mock.nu --test could not run: {}", e), tally);
            }
        }
    }

    let qemu = adapters_dir.join("qemu.nu");
    if qemu.exists() {
        check_qemu_base_image(tally);
    }

    println!();
}

fn check_qemu_base_image(tally: &mut Tally) {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        print_warn(
            "HOME is not set; skipping QEMU base image check (~/.bop/qemu-base.qcow2)",
            tally,
        );
        return;
    }

    let base_image = Path::new(&home).join(".bop").join("qemu-base.qcow2");
    if base_image.exists() {
        print_ok(format!("{} exists", base_image.display()));
    } else {
        print_warn(
            format!(
                "{} missing (build with: nu scripts/build-qemu-base.nu)",
                base_image.display()
            ),
            tally,
        );
    }
}

fn provider_kind(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("claude") {
        Some("claude")
    } else if lower.starts_with("codex") {
        Some("codex")
    } else if lower.starts_with("gemini") {
        Some("gemini")
    } else if lower.starts_with("ollama") {
        Some("ollama")
    } else {
        None
    }
}

fn check_providers(provider_names: Option<Vec<String>>, tally: &mut Tally) {
    println!("  Providers");

    let Some(mut names) = provider_names else {
        print_warn(
            "providers unavailable (providers.json missing or invalid)",
            tally,
        );
        println!();
        return;
    };

    names.sort();

    if names.is_empty() {
        print_warn("no registered providers in providers.json", tally);
        println!();
        return;
    }

    for name in names {
        match provider_kind(&name) {
            Some("claude") => {
                let home = std::env::var("HOME").unwrap_or_default();
                let file1 = Path::new(&home).join(".claude/credentials");
                let file2 = Path::new(&home).join(".claude/.credentials.json");
                let has_file = file1.exists() || file2.exists();
                let has_keychain = check_keychain_credential("Claude Code-credentials");

                if has_file || has_keychain {
                    print_ok(format!("{} credentials found", name));
                } else {
                    print_err(format!("{} credentials missing", name), tally);
                }
            }
            Some("codex") => {
                if !command_available("codex") {
                    print_warn(
                        format!("{} — CLI not installed, skipping credential check", name),
                        tally,
                    );
                } else {
                    let home = std::env::var("HOME").unwrap_or_default();
                    let auth = Path::new(&home).join(".codex/auth.json");
                    if auth.exists() {
                        print_ok(format!("{} credentials found", name));
                    } else {
                        print_err(
                            format!("{} credentials missing ({})", name, auth.display()),
                            tally,
                        );
                    }
                }
            }
            Some("gemini") => {
                let home = std::env::var("HOME").unwrap_or_default();
                let creds = Path::new(&home).join(".gemini/credentials.json");
                if creds.exists() {
                    print_ok(format!("{} credentials found", name));
                } else {
                    print_err(
                        format!("{} credentials missing ({})", name, creds.display()),
                        tally,
                    );
                }
            }
            Some("ollama") => match StdCommand::new("curl")
                .args(["-s", "--max-time", "2", "http://localhost:11434/api/tags"])
                .output()
            {
                Ok(out) if out.status.success() => {
                    print_ok(format!("{} responding at localhost:11434", name));
                }
                Ok(_) => {
                    print_err(format!("{} not responding at localhost:11434", name), tally);
                }
                Err(e) => {
                    print_err(format!("{} health check failed: {}", name, e), tally);
                }
            },
            _ => {
                print_ok(format!("{} no credential check required", name));
            }
        }
    }

    println!();
}

fn check_config(cards_root: &Path, tally: &mut Tally) {
    println!("  Config");

    let config_path = cards_root.join(".bop").join("config.json");
    if !config_path.exists() {
        print_ok(format!("{} not present (optional)", config_path.display()));
        println!();
        return;
    }

    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            print_err(
                format!("{} unreadable: {}", config_path.display(), e),
                tally,
            );
            println!();
            return;
        }
    };

    let value = match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(v) => {
            print_ok(format!("{} valid JSON", config_path.display()));
            v
        }
        Err(e) => {
            print_err(
                format!("{} invalid JSON: {}", config_path.display(), e),
                tally,
            );
            println!();
            return;
        }
    };

    match value.get("max_workers").and_then(|v| v.as_u64()) {
        Some(v) if v > 0 => print_ok(format!("max_workers = {}", v)),
        _ => print_err("max_workers must be a positive integer", tally),
    }

    println!();
}

pub fn cmd_doctor(cards_root: &Path, fast: bool, fix: bool) -> anyhow::Result<()> {
    println!("bop doctor");
    println!("{}", SEP);

    let mut tally = Tally::default();

    if fast {
        // Keep `--fast` backward-compatible; diagnostics are already lightweight.
        println!("  ⚠ --fast enabled: running lightweight diagnostics only");
        tally.warnings += 1;
        println!();
    }

    let nu_available = check_environment(&mut tally);
    let provider_names = check_filesystem(cards_root, fix, &mut tally);
    check_adapters(cards_root, nu_available, &mut tally);
    check_providers(provider_names, &mut tally);
    check_config(cards_root, &mut tally);

    println!("{}", SEP);
    println!("  {} errors, {} warnings", tally.errors, tally.warnings);
    if tally.errors > 0 && !fix {
        println!("  Run with --fix to auto-repair what's possible");
    }

    if tally.errors > 0 {
        anyhow::bail!("doctor found {} error(s)", tally.errors);
    }

    Ok(())
}

pub fn print_status_summary(root: &Path) -> anyhow::Result<()> {
    crate::list::list_cards(root, "active")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn command_available_cargo() {
        assert!(command_available("cargo"));
    }

    #[test]
    fn command_available_nonexistent() {
        assert!(!command_available("nonexistent-command-xyz-12345"));
    }

    #[test]
    fn semver_triplet_parses() {
        assert_eq!(parse_semver_triplet("0.111.0"), Some((0, 111, 0)));
        assert_eq!(parse_semver_triplet("nu 0.100.1 (abc)"), Some((0, 100, 1)));
    }

    #[test]
    fn print_status_summary_empty_root() {
        let td = tempdir().unwrap();
        for state in ["pending", "running", "done"] {
            fs::create_dir_all(td.path().join(state)).unwrap();
        }
        print_status_summary(td.path()).unwrap();
    }

    #[test]
    fn print_status_summary_with_cards() {
        let td = tempdir().unwrap();
        let card_dir = td.path().join("pending").join("test-card.bop");
        fs::create_dir_all(&card_dir).unwrap();
        let meta = bop_core::Meta {
            id: "test-card".into(),
            stage: "implement".into(),
            ..Default::default()
        };
        bop_core::write_meta(&card_dir, &meta).unwrap();
        print_status_summary(td.path()).unwrap();
    }

    #[test]
    fn check_filesystem_fix_creates_state_dirs_and_providers() {
        let td = tempdir().unwrap();
        fs::create_dir_all(td.path().join("templates/implement.bop")).unwrap();
        fs::create_dir_all(td.path().join(".locks")).unwrap();

        let mut tally = Tally::default();
        let providers = check_filesystem(td.path(), true, &mut tally);
        assert!(providers.is_some());
        assert_eq!(tally.errors, 0);

        for state in ["pending", "running", "done", "merged", "failed"] {
            assert!(td.path().join(state).is_dir());
        }
        assert!(td.path().join("providers.json").exists());
    }
}
