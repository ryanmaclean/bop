use anyhow::Context;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use jobcard_core::Meta;

pub const DEFAULT_MEMORY_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryStore {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entries: BTreeMap<String, MemoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub value: String,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MemoryOutput {
    Ops(MemoryOutputOps),
    Flat(BTreeMap<String, MemoryOutputValue>),
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MemoryOutputOps {
    #[serde(default)]
    pub set: BTreeMap<String, MemoryOutputValue>,
    #[serde(default)]
    pub delete: Vec<String>,
    #[serde(default)]
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MemoryOutputValue {
    String(String),
    Detailed {
        value: String,
        #[serde(default)]
        ttl_seconds: Option<i64>,
    },
}

fn normalize_namespace(namespace: &str) -> String {
    let trimmed = namespace.trim();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

fn sanitize_namespace(namespace: &str) -> String {
    let normalized = normalize_namespace(namespace);
    let sanitized: String = normalized
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "default".to_string()
    } else {
        sanitized
    }
}

fn memory_store_path(cards_dir: &Path, namespace: &str) -> PathBuf {
    cards_dir
        .join("memory")
        .join(format!("{}.json", sanitize_namespace(namespace)))
}

pub fn prune_memory_store(store: &mut MemoryStore, now: DateTime<Utc>) -> usize {
    let before = store.entries.len();
    store
        .entries
        .retain(|_, entry| entry.expires_at.map(|exp| exp > now).unwrap_or(true));
    before.saturating_sub(store.entries.len())
}

pub fn read_memory_store(cards_dir: &Path, namespace: &str) -> anyhow::Result<MemoryStore> {
    let namespace = normalize_namespace(namespace);
    let path = memory_store_path(cards_dir, &namespace);
    if !path.exists() {
        return Ok(MemoryStore::default());
    }

    let bytes = fs::read(&path)?;
    let mut store = if bytes.is_empty() {
        MemoryStore::default()
    } else {
        serde_json::from_slice::<MemoryStore>(&bytes)
            .with_context(|| format!("invalid memory store {}", path.display()))?
    };

    let pruned = prune_memory_store(&mut store, Utc::now());
    if pruned > 0 {
        write_memory_store(cards_dir, &namespace, &store)?;
    }

    Ok(store)
}

pub fn write_memory_store(
    cards_dir: &Path,
    namespace: &str,
    store: &MemoryStore,
) -> anyhow::Result<()> {
    fs::create_dir_all(cards_dir.join("memory"))?;
    let path = memory_store_path(cards_dir, namespace);
    let bytes = serde_json::to_vec_pretty(store)?;
    fs::write(path, bytes)?;
    Ok(())
}

pub fn set_memory_entry(
    store: &mut MemoryStore,
    key: &str,
    value: &str,
    ttl_seconds: i64,
    now: DateTime<Utc>,
) {
    let expires_at = now + ChronoDuration::seconds(ttl_seconds);
    store.entries.insert(
        key.to_string(),
        MemoryEntry {
            value: value.to_string(),
            updated_at: now,
            expires_at: Some(expires_at),
        },
    );
}

pub fn format_memory_for_prompt(store: &MemoryStore) -> String {
    if store.entries.is_empty() {
        return String::new();
    }

    let facts: BTreeMap<String, String> = store
        .entries
        .iter()
        .map(|(k, v)| (k.clone(), v.value.clone()))
        .collect();

    serde_json::to_string_pretty(&facts).unwrap_or_default()
}

pub fn memory_namespace_from_meta(meta: &Meta) -> String {
    meta.template_namespace
        .as_deref()
        .map(normalize_namespace)
        .filter(|ns| !ns.is_empty())
        .unwrap_or_else(|| normalize_namespace(&meta.stage))
}

pub fn parse_memory_output(path: &Path) -> anyhow::Result<MemoryOutputOps> {
    let bytes = fs::read(path)?;
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(MemoryOutputOps::default());
    }

    let parsed: MemoryOutput = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid memory output {}", path.display()))?;
    Ok(match parsed {
        MemoryOutput::Ops(ops) => ops,
        MemoryOutput::Flat(set) => MemoryOutputOps {
            set,
            delete: vec![],
            ttl_seconds: None,
        },
    })
}

pub fn merge_memory_output(cards_dir: &Path, namespace: &str, path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let ops = parse_memory_output(path)?;
    if ops.set.is_empty() && ops.delete.is_empty() {
        return Ok(());
    }

    let mut store = read_memory_store(cards_dir, namespace)?;
    let now = Utc::now();

    for key in ops.delete {
        let key = key.trim();
        if !key.is_empty() {
            store.entries.remove(key);
        }
    }

    for (key, value) in ops.set {
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let (value, item_ttl) = match value {
            MemoryOutputValue::String(v) => (v, None),
            MemoryOutputValue::Detailed { value, ttl_seconds } => (value, ttl_seconds),
        };
        let ttl_seconds = item_ttl
            .or(ops.ttl_seconds)
            .filter(|ttl| *ttl > 0)
            .unwrap_or(DEFAULT_MEMORY_TTL_SECONDS);
        set_memory_entry(&mut store, key, &value, ttl_seconds, now);
    }

    let _ = prune_memory_store(&mut store, now);
    write_memory_store(cards_dir, namespace, &store)?;
    Ok(())
}
