use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

use super::{Provider, ProviderSnapshot};

const OLLAMA_LOCAL_VERSION_URL: &str = "http://localhost:11434/api/version";
const OLLAMA_LOCAL_PS_URL: &str = "http://localhost:11434/api/ps";
const OLLAMA_CLOUD_PS_URL: &str = "https://ollama.com/api/ps";
const DETECT_TIMEOUT_MS: u64 = 500;
const FETCH_TIMEOUT_S: u64 = 5;

#[derive(Debug, Clone, Deserialize)]
pub struct OllamaCloudConfig {
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct OllamaModel {
    pub name: String,
    #[serde(default)]
    pub size_vram: Option<u64>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct OllamaPsResponse {
    #[serde(default)]
    pub models: Vec<OllamaModel>,
}

fn parse_ps_response(body: &str, provider: &str, display_name: &str) -> ProviderSnapshot {
    let ps: OllamaPsResponse = match serde_json::from_str(body) {
        Ok(parsed) => parsed,
        Err(e) => {
            return ProviderSnapshot {
                provider: provider.to_string(),
                display_name: display_name.to_string(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: None,
                secondary_label: None,
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "http".into(),
                error: Some(format!("failed to parse /api/ps JSON: {e}")),
                loaded_models: None,
            };
        }
    };

    // Keep this for future VRAM utilization display once total VRAM is exposed.
    let _loaded_vram_bytes: u64 = ps.models.iter().filter_map(|model| model.size_vram).sum();

    let loaded_models: Vec<String> = ps.models.iter().map(|model| model.name.clone()).collect();
    let reset_at = ps
        .models
        .iter()
        .filter_map(|model| model.expires_at.as_ref())
        .filter_map(|expires_at| DateTime::parse_from_rfc3339(expires_at).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .min();

    ProviderSnapshot {
        provider: provider.to_string(),
        display_name: display_name.to_string(),
        primary_pct: None,
        secondary_pct: None,
        primary_label: None,
        secondary_label: None,
        tokens_used: None,
        cost_usd: None,
        reset_at,
        source: "http".into(),
        error: None,
        loaded_models: Some(loaded_models),
    }
}

pub struct OllamaLocalProvider;

impl OllamaLocalProvider {
    pub fn new() -> Self {
        Self
    }

    fn parse_ps_response(body: &str) -> ProviderSnapshot {
        parse_ps_response(body, "ollama-local", "Ollama (local)")
    }
}

#[async_trait]
impl Provider for OllamaLocalProvider {
    fn name(&self) -> &str {
        "ollama-local"
    }

    fn detect(&self) -> bool {
        super::run_detect_async(async {
            let client = reqwest::Client::new();
            match tokio::time::timeout(
                Duration::from_millis(DETECT_TIMEOUT_MS),
                client.get(OLLAMA_LOCAL_VERSION_URL).send(),
            )
            .await
            {
                Ok(Ok(resp)) => resp.status().is_success(),
                Ok(Err(_)) | Err(_) => false,
            }
        })
    }

    async fn fetch(&self) -> anyhow::Result<ProviderSnapshot> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(FETCH_TIMEOUT_S))
            .build()
            .context("failed to build HTTP client")?;

        let resp = client
            .get(OLLAMA_LOCAL_PS_URL)
            .send()
            .await
            .context("failed to GET /api/ps")?;

        if !resp.status().is_success() {
            return Ok(ProviderSnapshot {
                provider: "ollama-local".into(),
                display_name: "Ollama (local)".into(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: None,
                secondary_label: None,
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "http".into(),
                error: Some(format!("HTTP {}", resp.status())),
                loaded_models: None,
            });
        }

        let body = resp.text().await.context("failed to read response body")?;
        Ok(Self::parse_ps_response(&body))
    }
}

pub struct OllamaCloudProvider;

impl OllamaCloudProvider {
    pub fn new() -> Self {
        Self
    }

    fn config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".ollama").join("config.json"))
    }

    fn read_cloud_credentials() -> anyhow::Result<Option<String>> {
        if let Ok(key) = std::env::var("OLLAMA_API_KEY") {
            if !key.trim().is_empty() {
                return Ok(Some(key));
            }
        }

        if let Some(path) = Self::config_path() {
            if path.exists() {
                let raw = std::fs::read_to_string(&path)
                    .with_context(|| format!("cannot read Ollama config: {}", path.display()))?;
                let cfg: OllamaCloudConfig = serde_json::from_str(&raw)
                    .with_context(|| format!("malformed Ollama config at {}", path.display()))?;
                let key = cfg
                    .api_key
                    .and_then(|k| if k.trim().is_empty() { None } else { Some(k) });
                return Ok(key);
            }
        }

        Ok(None)
    }

    fn parse_ps_response(body: &str) -> ProviderSnapshot {
        parse_ps_response(body, "ollama-cloud", "Ollama (cloud)")
    }
}

#[async_trait]
impl Provider for OllamaCloudProvider {
    fn name(&self) -> &str {
        "ollama-cloud"
    }

    fn detect(&self) -> bool {
        Self::read_cloud_credentials().ok().flatten().is_some()
    }

    async fn fetch(&self) -> anyhow::Result<ProviderSnapshot> {
        let api_key = match Self::read_cloud_credentials()? {
            Some(key) => key,
            None => {
                return Ok(ProviderSnapshot {
                    provider: "ollama-cloud".into(),
                    display_name: "Ollama (cloud)".into(),
                    primary_pct: None,
                    secondary_pct: None,
                    primary_label: None,
                    secondary_label: None,
                    tokens_used: None,
                    cost_usd: None,
                    reset_at: None,
                    source: "http".into(),
                    error: Some("no API key found".into()),
                    loaded_models: None,
                });
            }
        };

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(FETCH_TIMEOUT_S))
            .build()
            .context("failed to build HTTP client")?;

        let resp = client
            .get(OLLAMA_CLOUD_PS_URL)
            .header("Authorization", format!("Bearer {api_key}"))
            .send()
            .await
            .context("failed to GET /api/ps")?;

        if !resp.status().is_success() {
            return Ok(ProviderSnapshot {
                provider: "ollama-cloud".into(),
                display_name: "Ollama (cloud)".into(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: None,
                secondary_label: None,
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "http".into(),
                error: Some(format!("HTTP {}", resp.status())),
                loaded_models: None,
            });
        }

        let body = resp.text().await.context("failed to read response body")?;
        Ok(Self::parse_ps_response(&body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn parse_ps_response_with_models() {
        let json = r#"{
            "models": [
                {"name": "llama2:7b", "size_vram": 4294967296},
                {"name": "codellama:13b", "size_vram": 8589934592}
            ]
        }"#;
        let snap = OllamaLocalProvider::parse_ps_response(json);
        assert_eq!(snap.provider, "ollama-local");
        assert_eq!(snap.display_name, "Ollama (local)");
        assert_eq!(snap.source, "http");
        assert!(snap.error.is_none());
        let models = snap.loaded_models.unwrap();
        assert_eq!(models, vec!["llama2:7b", "codellama:13b"]);
    }

    #[test]
    fn parse_ps_response_with_empty_models() {
        let snap = OllamaLocalProvider::parse_ps_response(r#"{"models": []}"#);
        assert_eq!(snap.provider, "ollama-local");
        assert!(snap.error.is_none());
        assert_eq!(snap.loaded_models.unwrap(), Vec::<String>::new());
    }

    #[test]
    fn parse_ps_response_with_expires_at() {
        let json = r#"{
            "models": [
                {"name": "llama2:7b", "expires_at": "2026-03-07T12:00:00Z"},
                {"name": "codellama:13b", "expires_at": "2026-03-07T13:30:00Z"}
            ]
        }"#;
        let snap = OllamaLocalProvider::parse_ps_response(json);
        let reset = snap.reset_at.expect("reset_at should be set");
        assert_eq!(reset.to_rfc3339(), "2026-03-07T12:00:00+00:00");
    }

    #[test]
    fn parse_ps_response_with_malformed_json() {
        let snap = OllamaLocalProvider::parse_ps_response(r#"{"models": [{"name": "llama2""#);
        assert_eq!(snap.provider, "ollama-local");
        assert!(snap.error.is_some());
    }

    #[test]
    fn cloud_parse_ps_response_with_models() {
        let json = r#"{
            "models": [
                {"name": "mistral:latest"},
                {"name": "llama3:8b"}
            ]
        }"#;
        let snap = OllamaCloudProvider::parse_ps_response(json);
        assert_eq!(snap.provider, "ollama-cloud");
        assert_eq!(snap.display_name, "Ollama (cloud)");
        assert_eq!(
            snap.loaded_models.unwrap(),
            vec!["mistral:latest", "llama3:8b"]
        );
    }

    #[test]
    fn cloud_detect_with_env_var() {
        let _lock = ENV_MUTEX.lock().expect("lock env mutex");
        std::env::set_var("OLLAMA_API_KEY", "test-key");

        let provider = OllamaCloudProvider::new();
        assert!(provider.detect());

        std::env::remove_var("OLLAMA_API_KEY");
    }

    #[test]
    fn cloud_detect_without_credentials() {
        let _lock = ENV_MUTEX.lock().expect("lock env mutex");
        let td = tempdir().expect("create tempdir");
        let original_home = std::env::var("HOME").ok();

        std::env::remove_var("OLLAMA_API_KEY");
        std::env::set_var("HOME", td.path());

        let provider = OllamaCloudProvider::new();
        assert!(!provider.detect());

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn cloud_read_credentials_from_env_var() {
        let _lock = ENV_MUTEX.lock().expect("lock env mutex");
        std::env::set_var("OLLAMA_API_KEY", "env-var-key");

        let creds = OllamaCloudProvider::read_cloud_credentials().expect("read credentials");
        assert_eq!(creds, Some("env-var-key".to_string()));

        std::env::remove_var("OLLAMA_API_KEY");
    }

    #[test]
    fn cloud_credentials_parsing() {
        let json = r#"{"api_key": "test-key-from-config"}"#;
        let config: OllamaCloudConfig = serde_json::from_str(json).expect("parse config with key");
        assert_eq!(config.api_key, Some("test-key-from-config".to_string()));

        let json = r#"{}"#;
        let config: OllamaCloudConfig = serde_json::from_str(json).expect("parse config no key");
        assert_eq!(config.api_key, None);

        let json = r#"{"api_key": null}"#;
        let config: OllamaCloudConfig = serde_json::from_str(json).expect("parse config null key");
        assert_eq!(config.api_key, None);
    }
}
