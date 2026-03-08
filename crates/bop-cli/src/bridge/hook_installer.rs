use anyhow::Context;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

const CLAUDE_HOOK_EVENTS: [&str; 4] = ["SessionStart", "PreToolUse", "PostToolUse", "Stop"];
const BRIDGE_MARKER: &str = "bop bridge emit";

/// Install Claude Code hooks that emit BridgeEvents.
///
/// Writes (or merges) into `~/.claude/settings.json`:
/// - SessionStart → `bop bridge emit ... --event session-start ...`
/// - PreToolUse   → `bop bridge emit ... --event tool-start ...`
/// - PostToolUse  → `bop bridge emit ... --event tool-done ...`
/// - Stop         → `bop bridge emit ... --event session-end ...`
pub fn install_claude_hooks(bop_bin: &Path) -> anyhow::Result<()> {
    let settings_path = claude_settings_path().context("cannot resolve ~/.claude/settings.json")?;
    install_claude_hooks_at(&settings_path, bop_bin)
}

/// Remove bop hooks from `~/.claude/settings.json`.
pub fn uninstall_claude_hooks() -> anyhow::Result<()> {
    let settings_path = claude_settings_path().context("cannot resolve ~/.claude/settings.json")?;
    uninstall_claude_hooks_at(&settings_path)
}

fn claude_settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

fn install_claude_hooks_at(settings_path: &Path, bop_bin: &Path) -> anyhow::Result<()> {
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let mut settings = read_settings_json(settings_path)?;
    let root = ensure_object(&mut settings);
    let hooks = root.entry("hooks".to_string()).or_insert_with(|| json!({}));
    let hooks_obj = ensure_object(hooks);

    let cmd = |event: &str, extra: &str| -> String {
        let bin = bop_bin.display();
        if extra.is_empty() {
            format!("{bin} bridge emit --cli claude --event {event}")
        } else {
            format!("{bin} bridge emit --cli claude --event {event} {extra}")
        }
    };

    append_hook(
        hooks_obj,
        "SessionStart",
        cmd("session-start", "--session $SESSION_ID"),
    );
    append_hook(
        hooks_obj,
        "PreToolUse",
        cmd("tool-start", "--session $SESSION_ID --tool $TOOL_NAME"),
    );
    append_hook(
        hooks_obj,
        "PostToolUse",
        cmd("tool-done", "--session $SESSION_ID --tool $TOOL_NAME"),
    );
    append_hook(
        hooks_obj,
        "Stop",
        cmd("session-end", "--session $SESSION_ID"),
    );

    write_settings_json(settings_path, &settings)
}

fn uninstall_claude_hooks_at(settings_path: &Path) -> anyhow::Result<()> {
    if !settings_path.exists() {
        return Ok(());
    }

    let mut settings = read_settings_json(settings_path)?;
    let mut changed = false;

    if let Some(root) = settings.as_object_mut() {
        if let Some(hooks_value) = root.get_mut("hooks") {
            if let Some(hooks_obj) = hooks_value.as_object_mut() {
                for event in CLAUDE_HOOK_EVENTS {
                    let Some(entry) = hooks_obj.get_mut(event) else {
                        continue;
                    };
                    let Some(arr) = entry.as_array_mut() else {
                        continue;
                    };

                    let before = arr.len();
                    arr.retain(|hook| !is_bop_hook(hook));
                    if arr.len() != before {
                        changed = true;
                    }
                }

                hooks_obj.retain(|_, value| value.as_array().is_none_or(|arr| !arr.is_empty()));
                if hooks_obj.is_empty() {
                    root.remove("hooks");
                    changed = true;
                }
            }
        }
    }

    if changed {
        write_settings_json(settings_path, &settings)?;
    }
    Ok(())
}

fn read_settings_json(path: &Path) -> anyhow::Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }

    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }

    serde_json::from_str(&raw).with_context(|| format!("malformed JSON at {}", path.display()))
}

fn write_settings_json(path: &Path, settings: &Value) -> anyhow::Result<()> {
    let mut text =
        serde_json::to_string_pretty(settings).context("failed to serialize settings JSON")?;
    text.push('\n');
    fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
}

fn ensure_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    // Safe because we just normalized non-object values to object.
    value.as_object_mut().expect("value must be object")
}

fn append_hook(hooks: &mut Map<String, Value>, event: &str, command: String) {
    let new_hook = json!({
        "type": "command",
        "command": command,
    });

    let slot = hooks
        .entry(event.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));

    if let Some(arr) = slot.as_array_mut() {
        arr.push(new_hook);
        return;
    }

    let existing = std::mem::take(slot);
    *slot = Value::Array(vec![existing, new_hook]);
}

fn is_bop_hook(value: &Value) -> bool {
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "command")
        && value
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|cmd| cmd.contains(BRIDGE_MARKER))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_install_into_empty_settings() {
        let td = tempdir().unwrap();
        let settings_path = td.path().join(".claude").join("settings.json");

        install_claude_hooks_at(&settings_path, Path::new("/usr/local/bin/bop")).unwrap();

        let settings: Value =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert!(settings["hooks"]["SessionStart"].is_array());
        assert!(settings["hooks"]["PreToolUse"].is_array());
        assert!(settings["hooks"]["PostToolUse"].is_array());
        assert!(settings["hooks"]["Stop"].is_array());
        assert!(settings["hooks"]["SessionStart"][0]["command"]
            .as_str()
            .unwrap()
            .contains("bop bridge emit"));
    }

    #[test]
    fn test_install_merges_existing_settings() {
        let td = tempdir().unwrap();
        let settings_path = td.path().join(".claude").join("settings.json");
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        fs::write(
            &settings_path,
            r#"{"model":"opus","theme":"dark","nested":{"x":1}}"#,
        )
        .unwrap();

        install_claude_hooks_at(&settings_path, Path::new("/opt/bop")).unwrap();

        let settings: Value =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(settings["model"], "opus");
        assert_eq!(settings["theme"], "dark");
        assert_eq!(settings["nested"]["x"], 1);
        assert!(settings["hooks"]["SessionStart"].is_array());
    }

    #[test]
    fn test_install_appends_existing_hooks() {
        let td = tempdir().unwrap();
        let settings_path = td.path().join(".claude").join("settings.json");
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        fs::write(
            &settings_path,
            r#"{
  "hooks": {
    "SessionStart": [
      {"type":"command","command":"echo existing"}
    ]
  }
}"#,
        )
        .unwrap();

        install_claude_hooks_at(&settings_path, Path::new("/usr/bin/bop")).unwrap();

        let settings: Value =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        let arr = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["command"], "echo existing");
        assert!(arr[1]["command"]
            .as_str()
            .unwrap()
            .contains("bop bridge emit"));
    }

    #[test]
    fn test_uninstall_removes_bop_hooks_only() {
        let td = tempdir().unwrap();
        let settings_path = td.path().join(".claude").join("settings.json");
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        fs::write(
            &settings_path,
            r#"{
  "hooks": {
    "SessionStart": [
      {"type":"command","command":"echo keep-me"},
      {"type":"command","command":"/usr/bin/bop bridge emit --cli claude --event session-start"}
    ],
    "PreToolUse": [
      {"type":"command","command":"/usr/bin/bop bridge emit --cli claude --event tool-start"}
    ],
    "Stop": [
      {"type":"command","command":"echo other-hook"}
    ]
  },
  "theme": "dark"
}"#,
        )
        .unwrap();

        uninstall_claude_hooks_at(&settings_path).unwrap();

        let settings: Value =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        let session_start = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(session_start.len(), 1);
        assert_eq!(session_start[0]["command"], "echo keep-me");

        assert!(settings["hooks"].get("PreToolUse").is_none());
        assert_eq!(settings["hooks"]["Stop"][0]["command"], "echo other-hook");
        assert_eq!(settings["theme"], "dark");
    }
}
