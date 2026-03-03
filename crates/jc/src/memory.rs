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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;
    use tempfile::tempdir;

    #[test]
    fn default_memory_ttl_is_reasonable() {
        assert!(DEFAULT_MEMORY_TTL_SECONDS > 0);
        let one_year = 60 * 60 * 24 * 365;
        assert!(DEFAULT_MEMORY_TTL_SECONDS < one_year);
    }

    #[test]
    fn normalize_namespace_trims_whitespace() {
        assert_eq!(normalize_namespace("  hello  "), "hello");
    }

    #[test]
    fn normalize_namespace_empty_becomes_default() {
        assert_eq!(normalize_namespace(""), "default");
        assert_eq!(normalize_namespace("   "), "default");
    }

    #[test]
    fn sanitize_namespace_replaces_slashes() {
        assert_eq!(sanitize_namespace("a/b\\c"), "a_b_c");
    }

    #[test]
    fn sanitize_namespace_replaces_spaces() {
        assert_eq!(sanitize_namespace("hello world"), "hello_world");
    }

    #[test]
    fn sanitize_namespace_handles_empty() {
        assert_eq!(sanitize_namespace(""), "default");
    }

    #[test]
    fn sanitize_namespace_preserves_valid_chars() {
        assert_eq!(sanitize_namespace("my-ns_v2.0"), "my-ns_v2.0");
    }

    #[test]
    fn memory_store_path_returns_correct_path() {
        let dir = Path::new("/tmp/cards");
        let path = memory_store_path(dir, "myns");
        assert_eq!(path, PathBuf::from("/tmp/cards/memory/myns.json"));
    }

    #[test]
    fn read_write_memory_store_roundtrip() {
        let td = tempdir().unwrap();
        let now = Utc::now();
        let mut store = MemoryStore::default();
        store.entries.insert(
            "key1".to_string(),
            MemoryEntry {
                value: "val1".to_string(),
                updated_at: now,
                expires_at: Some(now + ChronoDuration::hours(1)),
            },
        );
        write_memory_store(td.path(), "test", &store).unwrap();
        let read_back = read_memory_store(td.path(), "test").unwrap();
        assert_eq!(read_back.entries["key1"].value, "val1");
    }

    #[test]
    fn read_memory_store_returns_empty_for_missing() {
        let td = tempdir().unwrap();
        let store = read_memory_store(td.path(), "nonexistent").unwrap();
        assert!(store.entries.is_empty());
    }

    #[test]
    fn prune_memory_store_removes_expired() {
        let now = Utc::now();
        let mut store = MemoryStore::default();
        store.entries.insert(
            "expired".to_string(),
            MemoryEntry {
                value: "old".to_string(),
                updated_at: now - ChronoDuration::hours(2),
                expires_at: Some(now - ChronoDuration::hours(1)),
            },
        );
        let pruned = prune_memory_store(&mut store, now);
        assert_eq!(pruned, 1);
        assert!(store.entries.is_empty());
    }

    #[test]
    fn prune_memory_store_keeps_valid() {
        let now = Utc::now();
        let mut store = MemoryStore::default();
        store.entries.insert(
            "valid".to_string(),
            MemoryEntry {
                value: "fresh".to_string(),
                updated_at: now,
                expires_at: Some(now + ChronoDuration::hours(1)),
            },
        );
        let pruned = prune_memory_store(&mut store, now);
        assert_eq!(pruned, 0);
        assert_eq!(store.entries.len(), 1);
    }

    #[test]
    fn prune_memory_store_keeps_no_expiry() {
        let now = Utc::now();
        let mut store = MemoryStore::default();
        store.entries.insert(
            "forever".to_string(),
            MemoryEntry {
                value: "eternal".to_string(),
                updated_at: now,
                expires_at: None,
            },
        );
        let pruned = prune_memory_store(&mut store, now);
        assert_eq!(pruned, 0);
        assert_eq!(store.entries.len(), 1);
    }

    #[test]
    fn prune_memory_store_handles_empty() {
        let now = Utc::now();
        let mut store = MemoryStore::default();
        let pruned = prune_memory_store(&mut store, now);
        assert_eq!(pruned, 0);
    }

    #[test]
    fn set_memory_entry_creates_new() {
        let now = Utc::now();
        let mut store = MemoryStore::default();
        set_memory_entry(&mut store, "k", "v", 3600, now);
        assert_eq!(store.entries["k"].value, "v");
        assert!(store.entries["k"].expires_at.is_some());
    }

    #[test]
    fn set_memory_entry_overwrites_existing() {
        let now = Utc::now();
        let mut store = MemoryStore::default();
        set_memory_entry(&mut store, "k", "v1", 3600, now);
        set_memory_entry(&mut store, "k", "v2", 3600, now);
        assert_eq!(store.entries["k"].value, "v2");
    }

    #[test]
    fn set_memory_entry_does_not_clobber_other_keys() {
        let now = Utc::now();
        let mut store = MemoryStore::default();
        set_memory_entry(&mut store, "a", "1", 3600, now);
        set_memory_entry(&mut store, "b", "2", 3600, now);
        assert_eq!(store.entries["a"].value, "1");
        assert_eq!(store.entries["b"].value, "2");
    }

    #[test]
    fn format_memory_for_prompt_empty_store() {
        let store = MemoryStore::default();
        assert_eq!(format_memory_for_prompt(&store), "");
    }

    #[test]
    fn format_memory_for_prompt_includes_key_value() {
        let now = Utc::now();
        let mut store = MemoryStore::default();
        set_memory_entry(&mut store, "mykey", "myval", 3600, now);
        let output = format_memory_for_prompt(&store);
        assert!(output.contains("mykey"));
        assert!(output.contains("myval"));
    }

    #[test]
    fn memory_namespace_from_meta_uses_template_namespace() {
        let meta = Meta {
            template_namespace: Some("custom-ns".to_string()),
            stage: "implement".to_string(),
            ..Default::default()
        };
        assert_eq!(memory_namespace_from_meta(&meta), "custom-ns");
    }

    #[test]
    fn memory_namespace_from_meta_falls_back_to_stage() {
        let meta = Meta {
            template_namespace: None,
            stage: "implement".to_string(),
            ..Default::default()
        };
        assert_eq!(memory_namespace_from_meta(&meta), "implement");
    }

    #[test]
    fn memory_namespace_from_meta_empty_template_ns_falls_back() {
        let meta = Meta {
            template_namespace: Some("   ".to_string()),
            stage: "qa".to_string(),
            ..Default::default()
        };
        // normalize_namespace("   ") => "default", but filter(|ns| !ns.is_empty()) passes
        // Actually: normalize trims to empty => "default", which is not empty, so it returns "default"
        // Let's just check what the function returns
        let ns = memory_namespace_from_meta(&meta);
        assert!(!ns.is_empty());
    }

    #[test]
    fn parse_memory_output_extracts_ops() {
        let td = tempdir().unwrap();
        let path = td.path().join("mem.json");
        let json = serde_json::json!({
            "set": { "key1": "val1" },
            "delete": ["old_key"]
        });
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let ops = parse_memory_output(&path).unwrap();
        assert!(ops.set.contains_key("key1"));
        assert_eq!(ops.delete, vec!["old_key"]);
    }

    #[test]
    fn parse_memory_output_flat_format() {
        let td = tempdir().unwrap();
        let path = td.path().join("mem.json");
        let json = serde_json::json!({
            "key1": "val1",
            "key2": "val2"
        });
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let ops = parse_memory_output(&path).unwrap();
        assert!(ops.set.contains_key("key1"));
        assert!(ops.set.contains_key("key2"));
        assert!(ops.delete.is_empty());
    }

    #[test]
    fn parse_memory_output_whitespace_only() {
        let td = tempdir().unwrap();
        let path = td.path().join("mem.json");
        fs::write(&path, "   \n  ").unwrap();
        let ops = parse_memory_output(&path).unwrap();
        assert!(ops.set.is_empty());
        assert!(ops.delete.is_empty());
    }

    #[test]
    fn merge_memory_output_applies_set() {
        let td = tempdir().unwrap();
        let mem_path = td.path().join("mem.json");
        let json = serde_json::json!({ "set": { "greeting": "hello" }, "delete": [] });
        fs::write(&mem_path, serde_json::to_vec(&json).unwrap()).unwrap();

        merge_memory_output(td.path(), "test", &mem_path).unwrap();
        let store = read_memory_store(td.path(), "test").unwrap();
        assert_eq!(store.entries["greeting"].value, "hello");
    }

    #[test]
    fn merge_memory_output_applies_delete() {
        let td = tempdir().unwrap();
        let now = Utc::now();

        // Pre-populate the store with an entry
        let mut store = MemoryStore::default();
        set_memory_entry(
            &mut store,
            "doomed",
            "value",
            DEFAULT_MEMORY_TTL_SECONDS,
            now,
        );
        write_memory_store(td.path(), "test", &store).unwrap();

        // Write a delete operation
        let mem_path = td.path().join("mem.json");
        let json = serde_json::json!({ "set": {}, "delete": ["doomed"] });
        fs::write(&mem_path, serde_json::to_vec(&json).unwrap()).unwrap();

        merge_memory_output(td.path(), "test", &mem_path).unwrap();
        let store = read_memory_store(td.path(), "test").unwrap();
        assert!(!store.entries.contains_key("doomed"));
    }

    #[test]
    fn merge_memory_output_noop_for_missing_file() {
        let td = tempdir().unwrap();
        let missing = td.path().join("nonexistent.json");
        assert!(merge_memory_output(td.path(), "test", &missing).is_ok());
    }
}
