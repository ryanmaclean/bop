use anyhow::{bail, Context};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self};
use std::io::{self, IsTerminal, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::{paths, util};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Manifest {
    bop_version: String,
    exported_at: String,
    card_id: String,
    state: String,
    provider: String,
    cost_usd: f64,
    tokens: u64,
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> anyhow::Result<Self> {
        let nanos = Utc::now()
            .timestamp_nanos_opt()
            .unwrap_or_else(|| Utc::now().timestamp_micros() * 1000);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn is_tarball_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|name| name.ends_with(".tar.gz") || name.ends_with(".tgz"))
        .unwrap_or(false)
}

pub fn cmd_export(
    root: &Path,
    id: &str,
    out: Option<&Path>,
    strip_logs: bool,
    _strip_worktree: bool,
) -> anyhow::Result<()> {
    let card_dir = paths::find_card(root, id).with_context(|| format!("card not found: {id}"))?;
    let state = card_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let meta = bop_core::read_meta(&card_dir)?;

    let safe_id = sanitize_card_id(&meta.id);
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S");
    let out_path = match out {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir()?.join(format!("bop-export-{safe_id}-{timestamp}.tar.gz")),
    };
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let staging = TempDir::new("bop-export")?;
    let bundle_root = safe_id;
    let bundle_dir = staging.path().join(&bundle_root);
    fs::create_dir_all(&bundle_dir)?;

    copy_file_if_exists(&card_dir.join("meta.json"), &bundle_dir.join("meta.json"))?;
    copy_file_if_exists(&card_dir.join("spec.md"), &bundle_dir.join("spec.md"))?;
    copy_file_if_exists(&card_dir.join("prompt.md"), &bundle_dir.join("prompt.md"))?;

    let output_dir = card_dir.join("output");
    if output_dir.exists() {
        util::copy_dir_all(&output_dir, &bundle_dir.join("output"))?;
    }

    if !strip_logs {
        let logs_dir = card_dir.join("logs");
        if logs_dir.exists() {
            util::copy_dir_all(&logs_dir, &bundle_dir.join("logs"))?;
        }
    }

    let manifest = build_manifest(&meta, &state);
    let manifest_path = staging.path().join("MANIFEST.json");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    run_tar(
        [
            "-czf",
            out_path
                .to_str()
                .context("output path is not valid UTF-8")?,
            "-C",
            staging
                .path()
                .to_str()
                .context("staging path is not valid UTF-8")?,
            "MANIFEST.json",
            &bundle_root,
        ],
        "failed to create export tarball",
    )?;

    println!("exported {} -> {}", meta.id, out_path.display());
    Ok(())
}

pub fn cmd_import_bundle(root: &Path, tarball: &Path, force: bool) -> anyhow::Result<()> {
    if !tarball.exists() {
        bail!("tarball not found: {}", tarball.display());
    }

    fs::create_dir_all(root.join("done"))?;

    validate_tar_paths(tarball)?;

    let extract_dir = TempDir::new("bop-import")?;
    run_tar(
        [
            "-xzf",
            tarball
                .to_str()
                .context("tarball path is not valid UTF-8")?,
            "-C",
            extract_dir
                .path()
                .to_str()
                .context("extract path is not valid UTF-8")?,
        ],
        "failed to extract import tarball",
    )?;

    let manifest = read_manifest(extract_dir.path())?;
    let bundle_dir = pick_bundle_dir(extract_dir.path(), manifest.as_ref())?;
    let bundle_name = bundle_dir
        .file_name()
        .and_then(|n| n.to_str())
        .context("invalid bundle directory name")?;
    let dest_name = if bundle_name.ends_with(".bop") || bundle_name.ends_with(".jobcard") {
        bundle_name.to_string()
    } else {
        format!("{bundle_name}.bop")
    };

    let dest_dir = root.join("done").join(dest_name);
    if dest_dir.exists() && !force && !confirm_overwrite(&dest_dir)? {
        bail!(
            "import cancelled: destination exists (use --force to overwrite): {}",
            dest_dir.display()
        );
    }
    if dest_dir.exists() {
        fs::remove_dir_all(&dest_dir)
            .with_context(|| format!("failed to remove existing {}", dest_dir.display()))?;
    }

    move_dir(&bundle_dir, &dest_dir)?;

    println!("imported {} -> {}", tarball.display(), dest_dir.display());
    Ok(())
}

fn run_tar<const N: usize>(args: [&str; N], fail_msg: &str) -> anyhow::Result<()> {
    let output = Command::new("tar")
        .args(args)
        .output()
        .context("failed to spawn tar command")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("{fail_msg}: {}", stderr.trim());
}

fn validate_tar_paths(tarball: &Path) -> anyhow::Result<()> {
    let output = Command::new("tar")
        .arg("-tzf")
        .arg(tarball)
        .output()
        .context("failed to list tarball entries")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to inspect tarball: {}", stderr.trim());
    }

    let listing = String::from_utf8(output.stdout).context("tar listing is not valid UTF-8")?;
    for raw in listing.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let path = Path::new(line);
        ensure_safe_relative_path(path)?;
    }

    Ok(())
}

fn move_dir(src: &Path, dst: &Path) -> anyhow::Result<()> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            util::copy_dir_all(src, dst)?;
            fs::remove_dir_all(src)?;
            Ok(())
        }
    }
}

fn confirm_overwrite(dest_dir: &Path) -> anyhow::Result<bool> {
    if !io::stdin().is_terminal() {
        return Ok(false);
    }
    print!("card exists at {}. overwrite? [y/N]: ", dest_dir.display());
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let normalized = input.trim().to_ascii_lowercase();
    Ok(matches!(normalized.as_str(), "y" | "yes"))
}

fn read_manifest(root: &Path) -> anyhow::Result<Option<Manifest>> {
    let path = root.join("MANIFEST.json");
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).context("invalid MANIFEST.json in archive")?;
    Ok(Some(manifest))
}

fn pick_bundle_dir(root: &Path, manifest: Option<&Manifest>) -> anyhow::Result<PathBuf> {
    if let Some(m) = manifest {
        let base = sanitize_card_id(&m.card_id);
        let candidates = [root.join(&base), root.join(format!("{base}.bop"))];
        for candidate in candidates {
            if candidate.is_dir() {
                return Ok(candidate);
            }
        }
    }

    let mut dirs = fs::read_dir(root)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    dirs.sort();

    if dirs.is_empty() {
        bail!("archive does not contain a card directory");
    }
    if dirs.len() > 1 {
        bail!("archive contains multiple top-level directories");
    }
    Ok(dirs.remove(0))
}

fn ensure_safe_relative_path(path: &Path) -> anyhow::Result<()> {
    if path.is_absolute() {
        bail!("archive entry is absolute path: {}", path.display());
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::RootDir) {
            bail!("archive entry contains invalid path: {}", path.display());
        }
    }
    Ok(())
}

fn copy_file_if_exists(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst).with_context(|| format!("failed to copy {}", src.display()))?;
    Ok(())
}

fn sanitize_card_id(id: &str) -> String {
    id.chars()
        .map(|ch| if ch == '/' || ch == '\\' { '-' } else { ch })
        .collect()
}

fn build_manifest(meta: &bop_core::Meta, state: &str) -> Manifest {
    let last_run = meta.runs.last();
    let provider = last_run
        .map(|run| run.provider.trim())
        .filter(|provider| !provider.is_empty())
        .map(ToString::to_string)
        .or_else(|| meta.provider_chain.first().cloned())
        .unwrap_or_else(|| "unknown".to_string());
    let cost_usd = last_run.and_then(|run| run.cost_usd).unwrap_or(0.0);
    let tokens = last_run
        .map(|run| run.prompt_tokens.unwrap_or(0) + run.completion_tokens.unwrap_or(0))
        .unwrap_or(0);

    Manifest {
        bop_version: env!("CARGO_PKG_VERSION").to_string(),
        exported_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        card_id: meta.id.clone(),
        state: state.to_string(),
        provider,
        cost_usd,
        tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bop_core::{write_meta, Meta, RunRecord};
    use chrono::Utc;

    fn create_card(
        root: &Path,
        state: &str,
        dir_name: &str,
        card_id: &str,
    ) -> anyhow::Result<PathBuf> {
        let card_dir = root.join(state).join(dir_name);
        fs::create_dir_all(card_dir.join("logs"))?;
        fs::create_dir_all(card_dir.join("output"))?;
        fs::create_dir_all(card_dir.join("worktree"))?;

        fs::write(card_dir.join("spec.md"), "# Spec\n")?;
        fs::write(card_dir.join("prompt.md"), "# Prompt\n")?;
        fs::write(card_dir.join("output").join("result.md"), "ok\n")?;
        fs::write(card_dir.join("logs").join("stdout.log"), "stdout\n")?;
        fs::write(card_dir.join("logs").join("stderr.log"), "stderr\n")?;
        fs::write(card_dir.join("logs").join("events.jsonl"), "{}\n")?;

        let meta = Meta {
            id: card_id.to_string(),
            created: Utc::now(),
            stage: "done".to_string(),
            runs: vec![RunRecord {
                provider: "codex".to_string(),
                prompt_tokens: Some(1000),
                completion_tokens: Some(400),
                cost_usd: Some(0.18),
                ..Default::default()
            }],
            ..Default::default()
        };
        write_meta(&card_dir, &meta)?;

        Ok(card_dir)
    }

    fn archive_entries(path: &Path) -> anyhow::Result<Vec<String>> {
        let output = Command::new("tar")
            .arg("-tzf")
            .arg(path)
            .output()
            .context("failed to read tar entries")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("failed to read tar entries: {}", stderr.trim());
        }
        let mut names = String::from_utf8(output.stdout)
            .context("tar output not valid UTF-8")?
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        names.sort();
        Ok(names)
    }

    #[test]
    fn export_and_import_round_trip() {
        let src = tempfile::tempdir().unwrap();
        paths::ensure_cards_layout(src.path()).unwrap();
        create_card(
            src.path(),
            "done",
            "team-arch-spec-041.bop",
            "team-arch-spec-041",
        )
        .unwrap();

        let tarball = src.path().join("bundle.tar.gz");
        cmd_export(
            src.path(),
            "team-arch-spec-041",
            Some(&tarball),
            false,
            false,
        )
        .unwrap();
        assert!(tarball.exists());

        let entries = archive_entries(&tarball).unwrap();
        assert!(entries.iter().any(|entry| entry == "MANIFEST.json"));
        assert!(entries.iter().any(|entry| entry.ends_with("/meta.json")));
        assert!(entries.iter().any(|entry| entry.ends_with("/spec.md")));
        assert!(entries.iter().any(|entry| entry.ends_with("/prompt.md")));
        assert!(entries
            .iter()
            .any(|entry| entry.contains("/output/result.md")));
        assert!(entries
            .iter()
            .any(|entry| entry.contains("/logs/events.jsonl")));

        let dst = tempfile::tempdir().unwrap();
        paths::ensure_cards_layout(dst.path()).unwrap();
        cmd_import_bundle(dst.path(), &tarball, true).unwrap();

        let imported = dst.path().join("done").join("team-arch-spec-041.bop");
        assert!(imported.exists());
        assert!(imported.join("meta.json").exists());
        assert!(imported.join("output").join("result.md").exists());
        assert!(imported.join("logs").join("stdout.log").exists());
        assert!(imported.join("logs").join("events.jsonl").exists());

        let meta = bop_core::read_meta(&imported).unwrap();
        assert_eq!(meta.id, "team-arch-spec-041");
    }

    #[test]
    fn export_strip_logs_excludes_logs_dir() {
        let src = tempfile::tempdir().unwrap();
        paths::ensure_cards_layout(src.path()).unwrap();
        create_card(
            src.path(),
            "done",
            "team-arch-spec-041.bop",
            "team-arch-spec-041",
        )
        .unwrap();

        let tarball = src.path().join("bundle-strip-logs.tar.gz");
        cmd_export(
            src.path(),
            "team-arch-spec-041",
            Some(&tarball),
            true,
            false,
        )
        .unwrap();

        let entries = archive_entries(&tarball).unwrap();
        assert!(!entries.iter().any(|entry| entry.contains("/logs/")));
        assert!(entries.iter().any(|entry| entry == "MANIFEST.json"));
    }
}
