use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use jobcard_core::Meta;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub command: String,
    #[serde(default)]
    pub rate_limit_exit: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until_epoch_s: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Extra environment variables injected when spawning this provider's adapter.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub providers: BTreeMap<String, Provider>,
}

pub type ProviderSelection = (
    String,
    String,
    i32,
    BTreeMap<String, String>,
    Option<String>,
);

fn validate_provider(name: &str, p: &Provider) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("provider name cannot be empty");
    }
    if p.command.trim().is_empty() {
        anyhow::bail!("provider '{}': command/adapter cannot be empty", name);
    }
    Ok(())
}

fn providers_path(cards_dir: &Path) -> PathBuf {
    cards_dir.join("providers.json")
}

pub fn read_providers(cards_dir: &Path) -> anyhow::Result<ProvidersFile> {
    let p = providers_path(cards_dir);
    if !p.exists() {
        return Ok(ProvidersFile::default());
    }
    let bytes = fs::read(p)?;
    let pf: ProvidersFile = serde_json::from_slice(&bytes)?;
    for (name, provider) in &pf.providers {
        validate_provider(name, provider)?;
    }
    Ok(pf)
}

pub fn write_providers(cards_dir: &Path, pf: &ProvidersFile) -> anyhow::Result<()> {
    for (name, provider) in &pf.providers {
        validate_provider(name, provider)?;
    }
    let bytes = serde_json::to_vec_pretty(pf)?;
    fs::write(providers_path(cards_dir), bytes)?;
    Ok(())
}

pub fn seed_providers(cards_dir: &Path) -> anyhow::Result<()> {
    let p = providers_path(cards_dir);
    if p.exists() {
        return Ok(());
    }

    let mut pf = ProvidersFile {
        default_provider: Some("mock".to_string()),
        ..Default::default()
    };
    pf.providers.insert(
        "mock".to_string(),
        Provider {
            command: "adapters/mock.zsh".to_string(),
            rate_limit_exit: 75,
            cooldown_until_epoch_s: None,
            model: None,
            env: Default::default(),
        },
    );
    pf.providers.insert(
        "mock2".to_string(),
        Provider {
            command: "adapters/mock.zsh".to_string(),
            rate_limit_exit: 75,
            cooldown_until_epoch_s: None,
            model: None,
            env: Default::default(),
        },
    );
    write_providers(cards_dir, &pf)?;
    Ok(())
}

pub fn ensure_mock_provider_command(cards_dir: &Path, adapter: &str) -> anyhow::Result<()> {
    let mut pf = read_providers(cards_dir)?;
    if let Some(p) = pf.providers.get_mut("mock") {
        p.command = adapter.to_string();
    }
    write_providers(cards_dir, &pf)?;
    Ok(())
}

pub fn rotate_provider_chain(meta: &mut Meta) {
    if meta.provider_chain.len() <= 1 {
        return;
    }
    let first = meta.provider_chain.remove(0);
    meta.provider_chain.push(first);
}

pub fn select_provider(
    cards_dir: &Path,
    meta: Option<&mut Meta>,
    stage: &str,
) -> anyhow::Result<Option<ProviderSelection>> {
    let pf = read_providers(cards_dir)?;
    let now = Utc::now().timestamp();

    let avoid_provider = if stage == "qa" {
        meta.as_ref()
            .and_then(|m| m.stages.get("implement"))
            .and_then(|r| r.provider.clone())
    } else {
        None
    };

    let chain: Vec<String> = match meta {
        Some(m) => {
            if m.provider_chain.is_empty() {
                m.provider_chain = vec!["mock".to_string(), "mock2".to_string()];
            }
            m.provider_chain.clone()
        }
        None => vec!["mock".to_string(), "mock2".to_string()],
    };

    let mut fallback: Option<ProviderSelection> = None;
    for name in chain {
        let Some(p) = pf.providers.get(&name) else {
            continue;
        };
        if let Some(until) = p.cooldown_until_epoch_s {
            if until > now {
                continue;
            }
        }

        if let Some(ref avoid) = avoid_provider {
            if &name == avoid {
                if fallback.is_none() {
                    fallback = Some((
                        name,
                        p.command.clone(),
                        p.rate_limit_exit,
                        p.env.clone(),
                        p.model.clone(),
                    ));
                }
                continue;
            }
        }

        return Ok(Some((
            name,
            p.command.clone(),
            p.rate_limit_exit,
            p.env.clone(),
            p.model.clone(),
        )));
    }

    Ok(fallback)
}

pub fn set_provider_cooldown(
    cards_dir: &Path,
    provider: &str,
    cooldown_s: i64,
) -> anyhow::Result<()> {
    let mut pf = read_providers(cards_dir)?;
    let now = Utc::now().timestamp();
    if let Some(p) = pf.providers.get_mut(provider) {
        p.cooldown_until_epoch_s = Some(now + cooldown_s);
    }
    write_providers(cards_dir, &pf)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jobcard_core::{Meta, StageRecord, StageStatus};
    use tempfile::tempdir;

    fn mock_provider(command: &str) -> Provider {
        Provider {
            command: command.to_string(),
            rate_limit_exit: 75,
            cooldown_until_epoch_s: None,
            model: None,
            env: Default::default(),
        }
    }

    #[test]
    fn validate_provider_accepts_valid() {
        let p = mock_provider("adapters/mock.zsh");
        assert!(validate_provider("mock", &p).is_ok());
    }

    #[test]
    fn validate_provider_rejects_empty_command() {
        let p = mock_provider("");
        assert!(validate_provider("mock", &p).is_err());
    }

    #[test]
    fn validate_provider_rejects_whitespace_command() {
        let p = mock_provider("   ");
        assert!(validate_provider("mock", &p).is_err());
    }

    #[test]
    fn validate_provider_rejects_empty_name() {
        let p = mock_provider("adapters/mock.zsh");
        assert!(validate_provider("", &p).is_err());
    }

    #[test]
    fn providers_path_returns_correct_path() {
        let dir = Path::new("/tmp/cards");
        assert_eq!(
            providers_path(dir),
            PathBuf::from("/tmp/cards/providers.json")
        );
    }

    #[test]
    fn read_write_providers_roundtrip() {
        let td = tempdir().unwrap();
        let mut pf = ProvidersFile::default();
        pf.default_provider = Some("mock".to_string());
        pf.providers
            .insert("mock".to_string(), mock_provider("adapters/mock.zsh"));
        write_providers(td.path(), &pf).unwrap();

        let read_back = read_providers(td.path()).unwrap();
        assert_eq!(read_back.default_provider, Some("mock".to_string()));
        assert!(read_back.providers.contains_key("mock"));
        assert_eq!(read_back.providers["mock"].command, "adapters/mock.zsh");
    }

    #[test]
    fn read_providers_returns_default_for_missing_file() {
        let td = tempdir().unwrap();
        let pf = read_providers(td.path()).unwrap();
        assert!(pf.providers.is_empty());
        assert!(pf.default_provider.is_none());
    }

    #[test]
    fn read_providers_rejects_invalid_entry() {
        let td = tempdir().unwrap();
        let json = serde_json::json!({
            "providers": {
                "bad": { "command": "" }
            }
        });
        fs::write(
            td.path().join("providers.json"),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
        assert!(read_providers(td.path()).is_err());
    }

    #[test]
    fn seed_providers_creates_file() {
        let td = tempdir().unwrap();
        seed_providers(td.path()).unwrap();
        assert!(td.path().join("providers.json").exists());
        let pf = read_providers(td.path()).unwrap();
        assert!(pf.providers.contains_key("mock"));
        assert!(pf.providers.contains_key("mock2"));
        assert_eq!(pf.default_provider, Some("mock".to_string()));
    }

    #[test]
    fn seed_providers_is_idempotent() {
        let td = tempdir().unwrap();
        seed_providers(td.path()).unwrap();
        // Modify the file to detect if seed_providers overwrites
        let mut pf = read_providers(td.path()).unwrap();
        pf.default_provider = Some("changed".to_string());
        write_providers(td.path(), &pf).unwrap();
        seed_providers(td.path()).unwrap();
        // Should not have been overwritten
        let pf2 = read_providers(td.path()).unwrap();
        assert_eq!(pf2.default_provider, Some("changed".to_string()));
    }

    #[test]
    fn ensure_mock_provider_command_updates_mock() {
        let td = tempdir().unwrap();
        seed_providers(td.path()).unwrap();
        ensure_mock_provider_command(td.path(), "adapters/new.zsh").unwrap();
        let pf = read_providers(td.path()).unwrap();
        assert_eq!(pf.providers["mock"].command, "adapters/new.zsh");
    }

    #[test]
    fn rotate_provider_chain_moves_first_to_last() {
        let mut meta = Meta {
            provider_chain: vec!["a".into(), "b".into(), "c".into()],
            ..Default::default()
        };
        rotate_provider_chain(&mut meta);
        assert_eq!(meta.provider_chain, vec!["b", "c", "a"]);
    }

    #[test]
    fn rotate_provider_chain_noop_on_single() {
        let mut meta = Meta {
            provider_chain: vec!["only".into()],
            ..Default::default()
        };
        rotate_provider_chain(&mut meta);
        assert_eq!(meta.provider_chain, vec!["only"]);
    }

    #[test]
    fn rotate_provider_chain_noop_on_empty() {
        let mut meta = Meta {
            provider_chain: vec![],
            ..Default::default()
        };
        rotate_provider_chain(&mut meta);
        assert!(meta.provider_chain.is_empty());
    }

    #[test]
    fn select_provider_returns_first_available() {
        let td = tempdir().unwrap();
        let mut pf = ProvidersFile::default();
        pf.providers.insert("a".to_string(), mock_provider("cmd_a"));
        pf.providers.insert("b".to_string(), mock_provider("cmd_b"));
        write_providers(td.path(), &pf).unwrap();

        let mut meta = Meta {
            provider_chain: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        let result = select_provider(td.path(), Some(&mut meta), "implement").unwrap();
        assert!(result.is_some());
        let (name, cmd, _, _, _) = result.unwrap();
        assert_eq!(name, "a");
        assert_eq!(cmd, "cmd_a");
    }

    #[test]
    fn select_provider_skips_cooled_down() {
        let td = tempdir().unwrap();
        let future = Utc::now().timestamp() + 9999;
        let mut pf = ProvidersFile::default();
        let mut cooled = mock_provider("cmd_a");
        cooled.cooldown_until_epoch_s = Some(future);
        pf.providers.insert("a".to_string(), cooled);
        pf.providers.insert("b".to_string(), mock_provider("cmd_b"));
        write_providers(td.path(), &pf).unwrap();

        let mut meta = Meta {
            provider_chain: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        let result = select_provider(td.path(), Some(&mut meta), "implement").unwrap();
        let (name, _, _, _, _) = result.unwrap();
        assert_eq!(name, "b");
    }

    #[test]
    fn select_provider_returns_none_when_all_cooled_down() {
        let td = tempdir().unwrap();
        let future = Utc::now().timestamp() + 9999;
        let mut pf = ProvidersFile::default();
        let mut a = mock_provider("cmd_a");
        a.cooldown_until_epoch_s = Some(future);
        let mut b = mock_provider("cmd_b");
        b.cooldown_until_epoch_s = Some(future);
        pf.providers.insert("a".to_string(), a);
        pf.providers.insert("b".to_string(), b);
        write_providers(td.path(), &pf).unwrap();

        let mut meta = Meta {
            provider_chain: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        let result = select_provider(td.path(), Some(&mut meta), "implement").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn select_provider_avoids_qa_provider() {
        let td = tempdir().unwrap();
        let mut pf = ProvidersFile::default();
        pf.providers
            .insert("impl_prov".to_string(), mock_provider("cmd_impl"));
        pf.providers
            .insert("qa_prov".to_string(), mock_provider("cmd_qa"));
        write_providers(td.path(), &pf).unwrap();

        let mut stages = BTreeMap::new();
        stages.insert(
            "implement".to_string(),
            StageRecord {
                status: StageStatus::Done,
                agent: None,
                provider: Some("impl_prov".to_string()),
                duration_s: None,
                started: None,
                blocked_by: None,
            },
        );
        let mut meta = Meta {
            provider_chain: vec!["impl_prov".into(), "qa_prov".into()],
            stages,
            ..Default::default()
        };
        let result = select_provider(td.path(), Some(&mut meta), "qa").unwrap();
        let (name, _, _, _, _) = result.unwrap();
        assert_eq!(name, "qa_prov");
    }

    #[test]
    fn select_provider_falls_back_to_avoided_if_only_option() {
        let td = tempdir().unwrap();
        let mut pf = ProvidersFile::default();
        pf.providers
            .insert("only_prov".to_string(), mock_provider("cmd"));
        write_providers(td.path(), &pf).unwrap();

        let mut stages = BTreeMap::new();
        stages.insert(
            "implement".to_string(),
            StageRecord {
                status: StageStatus::Done,
                agent: None,
                provider: Some("only_prov".to_string()),
                duration_s: None,
                started: None,
                blocked_by: None,
            },
        );
        let mut meta = Meta {
            provider_chain: vec!["only_prov".into()],
            stages,
            ..Default::default()
        };
        let result = select_provider(td.path(), Some(&mut meta), "qa").unwrap();
        let (name, _, _, _, _) = result.unwrap();
        assert_eq!(name, "only_prov");
    }

    #[test]
    fn set_provider_cooldown_sets_epoch() {
        let td = tempdir().unwrap();
        seed_providers(td.path()).unwrap();
        set_provider_cooldown(td.path(), "mock", 300).unwrap();
        let pf = read_providers(td.path()).unwrap();
        let cd = pf.providers["mock"].cooldown_until_epoch_s.unwrap();
        let now = Utc::now().timestamp();
        assert!(cd > now);
        assert!(cd <= now + 301);
    }
}
