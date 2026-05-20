//! Workspace initialisation logic.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::Path;
use walkdir::WalkDir;

/// Report produced by `init` in JSON mode.
#[derive(Debug, Serialize)]
pub struct InitReport {
    /// Directories created during init.
    pub created_dirs: Vec<String>,
    /// Path to the written `skillet.toml`.
    pub config_path: String,
}

/// Initialises a new skillet workspace at `workspace`.
///
/// When `json` is `true`, prints a JSON report to stdout instead of silence.
pub fn run(workspace: &Path, adopt: bool, json: bool) -> Result<()> {
    let config_path = workspace.join("skillet.toml");

    if config_path.exists() {
        bail!(
            "skillet.toml already exists at {}, refusing to overwrite",
            config_path.display()
        );
    }

    let default_cfg = crate::config::SkilletConfig::default().to_toml()?;

    let skills_src_dir = workspace.join("src/skills");
    let skills_out_dir = workspace.join("skills");
    let fragments_dir = workspace.join("src/skills/_fragments");

    if adopt {
        adopt_skills(&skills_out_dir, &skills_src_dir).context("failed to adopt SKILL.md files")?;
    }

    std::fs::create_dir_all(&skills_src_dir).context("failed to create skills source dir")?;
    std::fs::create_dir_all(&skills_out_dir).context("failed to create skills output dir")?;
    std::fs::create_dir_all(&fragments_dir).context("failed to create fragments dir")?;

    std::fs::write(&config_path, &default_cfg).context("failed to write skillet.toml")?;

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
                crate::workspace::copy_dir_recursive(sub_path, &dest_sub_dir)?;
            }
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
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn adopt_skills_copies_skill_md_as_named_dot_pan_file() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let skill_out_dir = tmp.path().join("diagnose");
        fs::create_dir_all(&skill_out_dir).unwrap();
        fs::write(skill_out_dir.join("SKILL.md"), "# Diagnose").unwrap();
        let skills_src = tmp.path().join("src/skills");

        // Act
        adopt_skills(tmp.path(), &skills_src).unwrap();

        // Assert
        let pan_file = skills_src.join("diagnose/diagnose.pan");
        assert!(pan_file.exists());
        assert_eq!(
            fs::read(skill_out_dir.join("SKILL.md")).unwrap(),
            fs::read(&pan_file).unwrap(),
        );
    }

    #[test]
    fn adopt_skills_preserves_original_skill_md() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let skill_out_dir = tmp.path().join("my-skill");
        fs::create_dir_all(&skill_out_dir).unwrap();
        fs::write(skill_out_dir.join("SKILL.md"), "# My Skill").unwrap();
        let skills_src = tmp.path().join("src/skills");

        // Act
        adopt_skills(tmp.path(), &skills_src).unwrap();

        // Assert — original is not removed
        assert!(skill_out_dir.join("SKILL.md").exists());
    }

    #[test]
    fn adopt_skills_is_noop_when_skills_dir_does_not_exist() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let nonexistent = tmp.path().join("nonexistent");
        let skills_src = tmp.path().join("src/skills");

        // Act & Assert — should not error
        assert!(adopt_skills(&nonexistent, &skills_src).is_ok());
    }

    #[test]
    fn adopt_skills_copies_reference_md_files_as_pan() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let skill_out_dir = tmp.path().join("my-skill");
        let ref_dir = skill_out_dir.join("reference");
        fs::create_dir_all(&ref_dir).unwrap();
        fs::write(skill_out_dir.join("SKILL.md"), "# My Skill").unwrap();
        fs::write(ref_dir.join("guide.md"), "# Guide").unwrap();
        let skills_src = tmp.path().join("src/skills");

        // Act
        adopt_skills(tmp.path(), &skills_src).unwrap();

        // Assert — reference/guide.md becomes reference/guide.pan in source
        let pan_ref = skills_src.join("my-skill/reference/guide.pan");
        assert!(pan_ref.exists(), "reference .pan file should be created");
        assert_eq!(fs::read_to_string(pan_ref).unwrap(), "# Guide");
    }

    #[test]
    fn adopt_skills_copies_other_subfolders_verbatim() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let skill_out_dir = tmp.path().join("my-skill");
        let scripts_dir = skill_out_dir.join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        fs::write(skill_out_dir.join("SKILL.md"), "# My Skill").unwrap();
        fs::write(scripts_dir.join("run.sh"), "#!/bin/sh").unwrap();
        let skills_src = tmp.path().join("src/skills");

        // Act
        adopt_skills(tmp.path(), &skills_src).unwrap();

        // Assert — scripts/run.sh is copied 1-to-1
        let dest = skills_src.join("my-skill/scripts/run.sh");
        assert!(dest.exists(), "scripts subfolder should be copied verbatim");
        assert_eq!(fs::read_to_string(dest).unwrap(), "#!/bin/sh");
    }

    #[test]
    fn adopt_reference_dir_renames_nested_md_to_pan() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.md"), "content a").unwrap();
        fs::write(src.join("sub/b.md"), "content b").unwrap();
        let dest = tmp.path().join("dest");

        // Act
        adopt_reference_dir(&src, &dest).unwrap();

        // Assert
        assert!(dest.join("a.pan").exists());
        assert!(dest.join("sub/b.pan").exists());
        assert!(!dest.join("a.md").exists());
    }

    #[test]
    fn run_creates_skills_dirs_and_config() {
        // Arrange
        let tmp = TempDir::new().unwrap();

        // Act
        run(tmp.path(), false, false).unwrap();

        // Assert
        assert!(tmp.path().join("src/skills").is_dir());
        assert!(tmp.path().join("src/skills/_fragments").is_dir());
        assert!(tmp.path().join("skills").is_dir());
        assert!(tmp.path().join("skillet.toml").is_file());
    }

    #[test]
    fn run_refuses_to_overwrite_existing_skillet_toml() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("skillet.toml"), "existing = true").unwrap();

        // Act
        let result = run(tmp.path(), false, false);

        // Assert
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("refusing to overwrite"));
    }
}
