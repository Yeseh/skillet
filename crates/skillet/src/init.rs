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
        .min_depth(2)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
            continue;
        }
        let parent = match path.parent() {
            Some(p) => p,
            None => continue,
        };
        let dir_name = match parent.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let dest_dir = skills_src_dir.join(&dir_name);
        std::fs::create_dir_all(&dest_dir)
            .with_context(|| format!("failed to create {}", dest_dir.display()))?;
        let dest = dest_dir.join(format!("{}.pan", dir_name));
        std::fs::copy(path, &dest)
            .with_context(|| format!("failed to copy {} to {}", path.display(), dest.display()))?;
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
