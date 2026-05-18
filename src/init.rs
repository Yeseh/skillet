//! Workspace initialisation logic.

use anyhow::{bail, Context, Result};
use std::path::Path;
use walkdir::WalkDir;

/// Initialises a new skillet workspace at `workspace`.
///
/// Creates the `skills/` and `skills/_fragments/` directories and writes a
/// default `skillet.toml`.  If `adopt` is `true`, any `SKILL.md` files found
/// directly inside `skills/<name>/` subdirectories are copied alongside their
/// parent directory as `<name>.skill` source files.
///
/// # Errors
///
/// Returns an error if `skillet.toml` already exists, or if any filesystem
/// operation fails.
pub fn run(workspace: &Path, adopt: bool) -> Result<()> {
    let config_path = workspace.join("skillet.toml");

    if config_path.exists() {
        bail!(
            "skillet.toml already exists at {}, refusing to overwrite",
            config_path.display()
        );
    }

    let default_cfg = crate::config::SkilletConfig::default().to_toml()?;

    let skills_dir = workspace.join("skills");
    let fragments_dir = workspace.join("skills/_fragments");

    if adopt {
        adopt_skills(&skills_dir).context("failed to adopt SKILL.md files")?;
    }

    std::fs::create_dir_all(&skills_dir).context("failed to create skills dir")?;
    std::fs::create_dir_all(&fragments_dir).context("failed to create fragments dir")?;

    std::fs::write(&config_path, &default_cfg).context("failed to write skillet.toml")?;

    Ok(())
}

fn adopt_skills(skills_dir: &Path) -> Result<()> {
    if !skills_dir.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(skills_dir)
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
        let dest = parent.join(format!("{}.skill", dir_name));
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
    fn adopt_skills_copies_skill_md_as_named_dot_skill_file() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("diagnose");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# Diagnose").unwrap();

        // Act
        adopt_skills(tmp.path()).unwrap();

        // Assert
        assert!(skill_dir.join("diagnose.skill").exists());
        assert_eq!(
            fs::read(skill_dir.join("SKILL.md")).unwrap(),
            fs::read(skill_dir.join("diagnose.skill")).unwrap(),
        );
    }

    #[test]
    fn adopt_skills_preserves_original_skill_md() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("my-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# My Skill").unwrap();

        // Act
        adopt_skills(tmp.path()).unwrap();

        // Assert — original is not removed
        assert!(skill_dir.join("SKILL.md").exists());
    }

    #[test]
    fn adopt_skills_is_noop_when_skills_dir_does_not_exist() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let nonexistent = tmp.path().join("nonexistent");

        // Act & Assert — should not error
        assert!(adopt_skills(&nonexistent).is_ok());
    }

    #[test]
    fn run_creates_skills_dir_fragments_dir_and_config() {
        // Arrange
        let tmp = TempDir::new().unwrap();

        // Act
        run(tmp.path(), false).unwrap();

        // Assert
        assert!(tmp.path().join("skills").is_dir());
        assert!(tmp.path().join("skills/_fragments").is_dir());
        assert!(tmp.path().join("skillet.toml").is_file());
    }

    #[test]
    fn run_refuses_to_overwrite_existing_skillet_toml() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("skillet.toml"), "existing = true").unwrap();

        // Act
        let result = run(tmp.path(), false);

        // Assert
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("refusing to overwrite"));
    }
}
