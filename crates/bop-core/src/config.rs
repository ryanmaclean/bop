use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// All fields are Option so partial configs can be merged.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Config {
    pub default_provider_chain: Option<Vec<String>>,
    pub max_concurrent: Option<usize>,
    pub cooldown_seconds: Option<u64>,
    pub log_retention_days: Option<u64>,
    pub default_template: Option<String>,
}

/// Merge `overlay` on top of `base`.  Non-None overlay values win.
pub fn merge_configs(base: Config, overlay: Config) -> Config {
    Config {
        default_provider_chain: overlay
            .default_provider_chain
            .or(base.default_provider_chain),
        max_concurrent: overlay.max_concurrent.or(base.max_concurrent),
        cooldown_seconds: overlay.cooldown_seconds.or(base.cooldown_seconds),
        log_retention_days: overlay.log_retention_days.or(base.log_retention_days),
        default_template: overlay.default_template.or(base.default_template),
    }
}

/// Parse JSON bytes into Config, returning a clear error on bad schema.
pub fn parse_config(json: &str) -> anyhow::Result<Config> {
    if json.trim().is_empty() {
        return Ok(Config::default());
    }
    serde_json::from_str(json).context(
        "malformed config: expected schema with optional fields: \
        default_provider_chain, max_concurrent, cooldown_seconds, \
        log_retention_days, default_template",
    )
}

/// Return the global config path: ~/.bop/config.json
pub fn global_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".bop").join("config.json"))
}

/// Return the project config path: <cwd>/.bop/config.json
pub fn project_config_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".bop")
        .join("config.json")
}

/// Migration warning: config.yaml → config.json
fn warn_if_stale_yaml(json_path: &std::path::Path) {
    let yaml_path = json_path.with_extension("yaml");
    if !json_path.exists() && yaml_path.exists() {
        eprintln!(
            "bop: config file '{}' found but '{}' not present. \
             Please rename your config file: mv {} {}",
            yaml_path.display(),
            json_path.display(),
            yaml_path.display(),
            json_path.display(),
        );
    }
}

/// Load and merge global + project configs.  Missing files are silently skipped.
/// Returns merged Config and emits a clear error for malformed JSON.
pub fn load_config() -> anyhow::Result<Config> {
    let global = match global_config_path() {
        Some(p) => {
            warn_if_stale_yaml(&p);
            if p.exists() {
                read_config_file(&p)
                    .with_context(|| format!("global config error: {}", p.display()))?
            } else {
                Config::default()
            }
        }
        None => Config::default(),
    };

    let project_path = project_config_path();
    warn_if_stale_yaml(&project_path);
    let project = if project_path.exists() {
        read_config_file(&project_path)
            .with_context(|| format!("project config error: {}", project_path.display()))?
    } else {
        Config::default()
    };

    Ok(merge_configs(global, project))
}

/// Read config from a specific path (used by `bop config get/set`).
pub fn read_config_file(path: &Path) -> anyhow::Result<Config> {
    let json = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read config: {}", path.display()))?;
    parse_config(&json).with_context(|| format!("invalid config at {}", path.display()))
}

/// Write config to a specific path (used by `bop config set`).
pub fn write_config_file(path: &Path, cfg: &Config) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create config dir: {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(cfg).context("failed to serialize config")?;
    std::fs::write(path, json)
        .with_context(|| format!("cannot write config: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_json() {
        let json = r#"{
  "default_provider_chain": ["claude", "codex"],
  "max_concurrent": 3,
  "cooldown_seconds": 120,
  "log_retention_days": 7,
  "default_template": "implement"
}"#;
        let cfg = parse_config(json).unwrap();
        assert_eq!(
            cfg.default_provider_chain,
            Some(vec!["claude".to_string(), "codex".to_string()])
        );
        assert_eq!(cfg.max_concurrent, Some(3));
        assert_eq!(cfg.cooldown_seconds, Some(120));
        assert_eq!(cfg.log_retention_days, Some(7));
        assert_eq!(cfg.default_template, Some("implement".to_string()));
    }

    #[test]
    fn parse_partial_json() {
        let json = r#"{"max_concurrent": 5}"#;
        let cfg = parse_config(json).unwrap();
        assert_eq!(cfg.max_concurrent, Some(5));
        assert_eq!(cfg.default_provider_chain, None);
    }

    #[test]
    fn parse_empty_json() {
        let cfg = parse_config("").unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn parse_malformed_json_returns_error() {
        let result = parse_config(r#"{"max_concurrent": "not_a_number"}"#);
        assert!(result.is_err(), "expected error for bad schema");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("max_concurrent") || msg.contains("invalid") || msg.contains("expected"),
            "error message should hint at the field: {}",
            msg
        );
    }

    #[test]
    fn merge_overlay_wins() {
        let base = Config {
            max_concurrent: Some(2),
            cooldown_seconds: Some(300),
            ..Default::default()
        };
        let overlay = Config {
            max_concurrent: Some(5),
            default_template: Some("qa".to_string()),
            ..Default::default()
        };
        let merged = merge_configs(base, overlay);
        assert_eq!(merged.max_concurrent, Some(5)); // overlay wins
        assert_eq!(merged.cooldown_seconds, Some(300)); // base kept
        assert_eq!(merged.default_template, Some("qa".to_string()));
    }

    #[test]
    fn merge_none_overlay_keeps_base() {
        let base = Config {
            max_concurrent: Some(4),
            ..Default::default()
        };
        let overlay = Config::default();
        let merged = merge_configs(base, overlay);
        assert_eq!(merged.max_concurrent, Some(4));
    }

    #[test]
    fn roundtrip_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let cfg = Config {
            max_concurrent: Some(2),
            default_template: Some("implement".to_string()),
            ..Default::default()
        };
        write_config_file(&path, &cfg).unwrap();
        let loaded = read_config_file(&path).unwrap();
        assert_eq!(loaded.max_concurrent, Some(2));
        assert_eq!(loaded.default_template, Some("implement".to_string()));
    }

    #[test]
    fn read_missing_file_returns_error() {
        let result = read_config_file(std::path::Path::new("/nonexistent/path/config.json"));
        assert!(result.is_err());
    }
}
