//! `skillet init` — workspace initialization and skill adoption.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::Path;
use walkdir::WalkDir;

/// Report produced by `init` in JSON mode.
#[derive(Debug, Serialize)]
pub struct InitReport {
    pub created_dirs: Vec<String>,
    pub config_path: String,
}

pub fn run(workspace: &Path, adopt: bool, json: bool) -> Result<()> {
    let config_path = workspace.join("skillet.toml");

    if config_path.exists() {
        bail!(
            "skillet.toml already exists at {}, refusing to overwrite",
            config_path.display()
        );
    }

    let module_name = derive_module_name(workspace);
    let config_toml = generate_config_toml(&module_name);

    let skills_src_dir = workspace.join("src/skills");
    let skills_out_dir = workspace.join("skills");
    let fragments_dir = workspace.join("src/skills/_fragments");

    if adopt {
        adopt_skills(&skills_out_dir, &skills_src_dir).context("failed to adopt SKILL.md files")?;
    }

    std::fs::create_dir_all(&skills_src_dir).context("failed to create skills source dir")?;
    std::fs::create_dir_all(&skills_out_dir).context("failed to create skills output dir")?;
    std::fs::create_dir_all(&fragments_dir).context("failed to create fragments dir")?;

    std::fs::write(&config_path, &config_toml).context("failed to write skillet.toml")?;

    if json {
        let report = InitReport {
            created_dirs: vec![
                skills_src_dir.to_string_lossy().to_string(),
                skills_out_dir.to_string_lossy().to_string(),
                fragments_dir.to_string_lossy().to_string(),
            ],
            config_path: config_path.to_string_lossy().to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(())
}

/// Derives a kebab-case module name from the workspace directory name.
fn derive_module_name(workspace: &Path) -> String {
    let folder = workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("my-module");
    to_kebab_case(folder)
}

/// Converts a string to kebab-case: lowercase, non-alphanumeric runs become `-`.
fn to_kebab_case(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut result = String::new();
    let mut prev_hyphen = true;
    for ch in lower.chars() {
        if ch.is_alphanumeric() {
            result.push(ch);
            prev_hyphen = false;
        } else if !prev_hyphen {
            result.push('-');
            prev_hyphen = true;
        }
    }
    let trimmed = result.trim_end_matches('-').to_string();
    if trimmed.is_empty() {
        "my-module".to_string()
    } else {
        trimmed
    }
}

/// Generates the `skillet.toml` content for a new workspace with one named module.
fn generate_config_toml(module_name: &str) -> String {
    format!(
        r#"[workspace]

[vars]
project_name = "{module_name}"

[lint]
max_activation_tokens = 4000
max_discovery_tokens = 100
max_fragment_tokens = 500

[build]
tokenizer = "cl100k_base"
verify_urls = false

[module.{module_name}]
src_dir = "src/skills"
out_dir = "skills"
version = "0.1.0"
fragments_dir = "src/skills/_fragments"
"#
    )
}

fn adopt_skills(skills_out_dir: &Path, skills_src_dir: &Path) -> Result<()> {
    if !skills_out_dir.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(skills_out_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let skill_out_dir = entry.path();
        if !skill_out_dir.is_dir() {
            continue;
        }
        let dir_name = match skill_out_dir.file_name().and_then(|n| n.to_str()) {
            Some(n) if !n.starts_with('_') && !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        let skill_md = skill_out_dir.join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }

        let dest_skill_dir = skills_src_dir.join(&dir_name);
        std::fs::create_dir_all(&dest_skill_dir)
            .with_context(|| format!("failed to create {}", dest_skill_dir.display()))?;

        let dest = dest_skill_dir.join(format!("{}.pan", dir_name));
        std::fs::copy(&skill_md, &dest).with_context(|| {
            format!(
                "failed to copy {} to {}",
                skill_md.display(),
                dest.display()
            )
        })?;

        for sub_entry in WalkDir::new(skill_out_dir)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let sub_path = sub_entry.path();
            if !sub_path.is_dir() {
                continue;
            }
            let sub_name = match sub_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let dest_sub_dir = dest_skill_dir.join(&sub_name);
            if sub_name == "reference" {
                adopt_reference_dir(sub_path, &dest_sub_dir)?;
            } else {
                copy_dir_recursive(sub_path, &dest_sub_dir)?;
            }
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap();
        if rel == std::path::Path::new("") {
            continue;
        }
        if path.is_dir() {
            std::fs::create_dir_all(dest.join(rel))
                .with_context(|| format!("failed to create {}", dest.join(rel).display()))?;
        } else {
            let dest_file = dest.join(rel);
            if let Some(parent) = dest_file.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            std::fs::copy(path, &dest_file).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    path.display(),
                    dest_file.display()
                )
            })?;
        }
    }
    Ok(())
}

fn adopt_reference_dir(src: &Path, dest: &Path) -> Result<()> {
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap();
        if rel == std::path::Path::new("") {
            continue;
        }
        if path.is_dir() {
            std::fs::create_dir_all(dest.join(rel))
                .with_context(|| format!("failed to create {}", dest.join(rel).display()))?;
        } else {
            let dest_file = if path.extension().and_then(|e| e.to_str()) == Some("md") {
                dest.join(rel.with_extension("pan"))
            } else {
                dest.join(rel)
            };
            if let Some(parent) = dest_file.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            std::fs::copy(path, &dest_file).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    path.display(),
                    dest_file.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_kebab_case_lowercases_and_replaces_separators() {
        assert_eq!(to_kebab_case("My Project"), "my-project");
        assert_eq!(to_kebab_case("MyPlugin2.0"), "myplugin2-0");
        assert_eq!(to_kebab_case("hello_world"), "hello-world");
        assert_eq!(to_kebab_case("already-kebab"), "already-kebab");
    }

    #[test]
    fn to_kebab_case_collapses_runs_of_separators() {
        assert_eq!(to_kebab_case("a  b--c"), "a-b-c");
    }

    #[test]
    fn to_kebab_case_trims_trailing_hyphen() {
        assert_eq!(to_kebab_case("foo!"), "foo");
    }

    #[test]
    fn generate_config_toml_contains_module_name() {
        let toml = generate_config_toml("my-skill-pack");
        assert!(toml.contains("[module.my-skill-pack]"));
        assert!(toml.contains("project_name = \"my-skill-pack\""));
    }
}
