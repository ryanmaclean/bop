use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const PROJECTS_FILE: &str = "projects.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectEntry {
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchProject {
    pub name: String,
    pub cards_root: PathBuf,
}

fn registry_path() -> anyhow::Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".bop").join(PROJECTS_FILE))
        .context("HOME is not set; cannot resolve ~/.bop/projects.json")
}

fn read_registry_from(path: &Path) -> anyhow::Result<Vec<ProjectEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read registry {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str::<Vec<ProjectEntry>>(&raw)
        .with_context(|| format!("invalid registry JSON at {}", path.display()))
}

fn write_registry_to(path: &Path, projects: &[ProjectEntry]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create registry dir {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(projects).context("failed to encode registry JSON")?;
    fs::write(path, format!("{json}\n"))
        .with_context(|| format!("failed to write registry {}", path.display()))
}

fn normalize_project_path(raw: &str) -> anyhow::Result<PathBuf> {
    let candidate = PathBuf::from(raw);
    let absolute = if candidate.is_absolute() {
        candidate
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(candidate)
    };
    if !absolute.exists() {
        anyhow::bail!("project path does not exist: {}", absolute.display());
    }
    if !absolute.is_dir() {
        anyhow::bail!("project path is not a directory: {}", absolute.display());
    }
    Ok(fs::canonicalize(&absolute).unwrap_or(absolute))
}

fn project_name_from_path(path: &Path) -> anyhow::Result<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .context("failed to derive project name from path")
}

fn sort_projects(projects: &mut [ProjectEntry]) {
    projects.sort_by(|a, b| a.name.cmp(&b.name));
}

fn resolve_from_registry(projects: &[ProjectEntry], alias_or_path: &str) -> Option<PathBuf> {
    projects
        .iter()
        .find(|entry| {
            entry.name == alias_or_path
                || entry
                    .alias
                    .as_deref()
                    .is_some_and(|alias| alias == alias_or_path)
        })
        .map(|entry| PathBuf::from(&entry.path))
}

fn add_project_to_registry(
    registry: &Path,
    raw_path: &str,
    alias: Option<&str>,
) -> anyhow::Result<ProjectEntry> {
    let path = normalize_project_path(raw_path)?;
    let path_str = path.to_string_lossy().to_string();
    let name = project_name_from_path(&path)?;
    let alias = alias
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let mut projects = read_registry_from(registry)?;
    if let Some(alias_value) = alias.as_deref() {
        if projects.iter().any(|entry| {
            entry
                .alias
                .as_deref()
                .is_some_and(|existing| existing == alias_value)
                && entry.path != path_str
        }) {
            anyhow::bail!("alias already registered: {alias_value}");
        }
    }

    let mut updated = false;
    for entry in &mut projects {
        if entry.path == path_str || entry.name == name {
            entry.name = name.clone();
            entry.path = path_str.clone();
            entry.alias = alias.clone();
            updated = true;
            break;
        }
    }

    if !updated {
        projects.push(ProjectEntry {
            name: name.clone(),
            path: path_str.clone(),
            alias: alias.clone(),
        });
    }
    sort_projects(&mut projects);
    write_registry_to(registry, &projects)?;

    Ok(ProjectEntry {
        name,
        path: path_str,
        alias,
    })
}

fn remove_project_from_registry(registry: &Path, target: &str) -> anyhow::Result<ProjectEntry> {
    let mut projects = read_registry_from(registry)?;
    let normalized_target = normalize_project_path(target)
        .ok()
        .map(|path| path.to_string_lossy().to_string());

    let original_len = projects.len();
    let mut removed: Option<ProjectEntry> = None;
    projects.retain(|entry| {
        let matches = entry.name == target
            || entry.alias.as_deref().is_some_and(|alias| alias == target)
            || entry.path == target
            || normalized_target
                .as_deref()
                .is_some_and(|normalized| entry.path == normalized);
        if matches && removed.is_none() {
            removed = Some(entry.clone());
        }
        !matches
    });

    if original_len == projects.len() {
        anyhow::bail!("project not found: {target}");
    }

    sort_projects(&mut projects);
    write_registry_to(registry, &projects)?;
    removed.context("failed to determine removed project")
}

pub fn read_registry() -> anyhow::Result<Vec<ProjectEntry>> {
    let path = registry_path()?;
    read_registry_from(&path)
}

pub fn find_project(alias_or_path: &str) -> anyhow::Result<PathBuf> {
    let projects = read_registry()?;
    if let Some(path) = resolve_from_registry(&projects, alias_or_path) {
        return Ok(path);
    }
    Ok(PathBuf::from(alias_or_path))
}

pub fn registered_watch_projects() -> anyhow::Result<Vec<WatchProject>> {
    let mut projects = read_registry()?;
    sort_projects(&mut projects);
    Ok(projects
        .into_iter()
        .map(|project| WatchProject {
            name: project.name,
            cards_root: PathBuf::from(project.path).join(".cards"),
        })
        .collect())
}

pub fn cmd_project_add(path: &str, alias: Option<&str>) -> anyhow::Result<()> {
    let registry = registry_path()?;
    let added = add_project_to_registry(&registry, path, alias)?;
    let alias_suffix = added
        .alias
        .as_deref()
        .map(|value| format!(" (alias: {value})"))
        .unwrap_or_default();
    println!(
        "registered project {}{} -> {}",
        added.name, alias_suffix, added.path
    );
    Ok(())
}

pub fn cmd_project_list() -> anyhow::Result<()> {
    let mut projects = read_registry()?;
    sort_projects(&mut projects);
    if projects.is_empty() {
        println!("no projects registered");
        return Ok(());
    }

    println!("{:<16} {:<8} PATH", "NAME", "ALIAS");
    for project in projects {
        let alias = project.alias.unwrap_or_else(|| "-".to_string());
        println!("{:<16} {:<8} {}", project.name, alias, project.path);
    }
    Ok(())
}

pub fn cmd_project_remove(target: &str) -> anyhow::Result<()> {
    let registry = registry_path()?;
    let removed = remove_project_from_registry(&registry, target)?;
    println!("removed project {} ({})", removed.name, removed.path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_registry(path: &Path, projects: &[ProjectEntry]) {
        write_registry_to(path, projects).unwrap();
    }

    #[test]
    fn resolve_alias_or_name_then_fallback() {
        let projects = vec![
            ProjectEntry {
                name: "bop".to_string(),
                path: "/tmp/bop".to_string(),
                alias: Some("b".to_string()),
            },
            ProjectEntry {
                name: "efi".to_string(),
                path: "/tmp/efi".to_string(),
                alias: Some("e".to_string()),
            },
        ];

        assert_eq!(
            resolve_from_registry(&projects, "b"),
            Some(PathBuf::from("/tmp/bop"))
        );
        assert_eq!(
            resolve_from_registry(&projects, "efi"),
            Some(PathBuf::from("/tmp/efi"))
        );
        assert_eq!(resolve_from_registry(&projects, "/tmp/x"), None);
    }

    #[test]
    fn add_and_remove_project_registry() {
        let td = tempdir().unwrap();
        let registry = td.path().join("projects.json");
        let project_root = td.path().join("repo");
        fs::create_dir_all(&project_root).unwrap();

        let added =
            add_project_to_registry(&registry, project_root.to_str().unwrap(), Some("r")).unwrap();
        assert_eq!(added.name, "repo");
        assert_eq!(added.alias.as_deref(), Some("r"));

        let all = read_registry_from(&registry).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "repo");

        let removed = remove_project_from_registry(&registry, "r").unwrap();
        assert_eq!(removed.name, "repo");
        assert!(read_registry_from(&registry).unwrap().is_empty());
    }

    #[test]
    fn duplicate_alias_is_rejected() {
        let td = tempdir().unwrap();
        let registry = td.path().join("projects.json");
        let p1 = td.path().join("repo1");
        let p2 = td.path().join("repo2");
        fs::create_dir_all(&p1).unwrap();
        fs::create_dir_all(&p2).unwrap();
        make_registry(
            &registry,
            &[ProjectEntry {
                name: "repo1".to_string(),
                path: p1.to_string_lossy().to_string(),
                alias: Some("x".to_string()),
            }],
        );

        let err = add_project_to_registry(&registry, p2.to_str().unwrap(), Some("x")).unwrap_err();
        assert!(err.to_string().contains("alias already registered"));
    }
}
