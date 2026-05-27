//! Workspace discovery and I/O helpers for the CLI.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// A discovered skill source within the workspace.
#[allow(dead_code)]
pub struct WorkspaceSkill {
    pub name: String,
    pub source_path: PathBuf,
    pub skill_dir: PathBuf,
    pub skill_out_dir: PathBuf,
}

/// Scans `src_dir` one level deep, returning skills whose directories contain
/// a `{name}.pan` source file.  Skips dirs starting with `_` or `.`.
#[allow(dead_code)]
pub fn discover_skills(src_dir: &Path, out_dir: &Path) -> Result<Vec<WorkspaceSkill>> {
    let mut skills = Vec::new();

    if !src_dir.exists() {
        return Ok(skills);
    }

    for entry in WalkDir::new(src_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let skill_dir = entry.path();
        if !skill_dir.is_dir() {
            continue;
        }
        let dir_name = match skill_dir.file_name().and_then(|n| n.to_str()) {
            Some(n) if !n.starts_with('_') && !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        let source_path = skill_dir.join(format!("{}.pan", dir_name));
        if source_path.exists() {
            skills.push(WorkspaceSkill {
                name: dir_name.clone(),
                source_path,
                skill_dir: skill_dir.to_path_buf(),
                skill_out_dir: out_dir.join(&dir_name),
            });
        }
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

/// Reads the `.pan` source for a skill.
#[allow(dead_code)]
pub fn read_source(skill: &WorkspaceSkill) -> Result<String> {
    std::fs::read_to_string(&skill.source_path).with_context(|| {
        format!(
            "failed to read skill source '{}'",
            skill.source_path.display()
        )
    })
}

/// Reads all `*.fragment.pan` files from `fragments_dir`.
///
/// Returns a map of fragment name (without the `.fragment.pan` suffix) to content.
#[allow(dead_code)]
pub fn read_fragments(fragments_dir: &Path) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();

    if !fragments_dir.exists() {
        return Ok(map);
    }

    for entry in WalkDir::new(fragments_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        let name = match file_name.strip_suffix(".fragment.pan") {
            Some(n) => n.to_string(),
            None => continue,
        };
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read fragment '{}'", path.display()))?;
        map.insert(name, content);
    }

    Ok(map)
}

/// Recursively copies `src` into `dest`, preserving the directory tree.
pub fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap();
        if rel == Path::new("") {
            continue;
        }
        let target = dest.join(rel);
        if path.is_dir() {
            std::fs::create_dir_all(&target)
                .with_context(|| format!("failed to create directory {}", target.display()))?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory {}", parent.display()))?;
            }
            std::fs::copy(path, &target).with_context(|| {
                format!("failed to copy {} to {}", path.display(), target.display())
            })?;
        }
    }
    Ok(())
}
