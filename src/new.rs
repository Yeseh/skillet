//! `skillet new` — scaffold a new skill source inside an initialized workspace.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// Renders the minimal `.skill` scaffold for a skill named `name`.
pub fn scaffold_content(name: &str) -> String {
    format!("---\nname: {name}\ndescription: \"TODO: describe this skill\"\n---\n\n# {name}\n")
}

/// Scaffolds a new skill source at `<skills_dir>/<name>/<name>.skill`.
///
/// Returns an error if the workspace is not initialized or if the skill already exists.
pub fn run(workspace: &Path, name: &str) -> Result<()> {
    let config = crate::config::load(workspace)?;
    let skills_dir = workspace.join(&config.workspace.skills_dir);
    let skill_dir = skills_dir.join(name);
    let skill_file = skill_dir.join(format!("{name}.skill"));

    if skill_dir.exists() {
        bail!("skill '{name}' already exists at {}", skill_dir.display());
    }

    std::fs::create_dir_all(&skill_dir)
        .with_context(|| format!("failed to create directory {}", skill_dir.display()))?;

    std::fs::write(&skill_file, scaffold_content(name))
        .with_context(|| format!("failed to write {}", skill_file.display()))?;

    println!("created {}", skill_file.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use crate::config::SkilletConfig;
    use tempfile::TempDir;

    fn init_workspace(dir: &Path) {
        let config = SkilletConfig::default();
        fs::write(dir.join("skillet.toml"), config.to_toml().unwrap()).unwrap();
        fs::create_dir_all(dir.join(&config.workspace.skills_dir)).unwrap();
        fs::create_dir_all(dir.join(&config.workspace.fragments_dir)).unwrap();
    }

    #[test]
    fn scaffold_template_contains_name_empty_description_and_heading() {
        // Arrange
        let name = "my-skill";

        // Act
        let content = scaffold_content(name);

        // Assert
        assert!(
            content.contains("name: my-skill"),
            "should include name field"
        );
        assert!(
            content.contains("description:"),
            "should include empty description"
        );
        assert!(content.contains("# my-skill"), "should include heading");
    }

    #[test]
    fn run_creates_skill_directory_and_source_file() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());

        // Act
        run(tmp.path(), "my-skill").unwrap();

        // Assert
        let skill_file = tmp.path().join("skills/my-skill/my-skill.skill");
        assert!(skill_file.exists(), "skill file should be created");
        let content = fs::read_to_string(&skill_file).unwrap();
        assert!(content.contains("name: my-skill"));
    }

    #[test]
    fn run_refuses_to_overwrite_existing_skill_directory() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        run(tmp.path(), "dupe").unwrap();

        // Act
        let result = run(tmp.path(), "dupe");

        // Assert
        assert!(result.is_err(), "should fail on duplicate");
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("dupe"), "error should mention skill name");
        assert!(
            msg.contains("already exists"),
            "error should say already exists"
        );
    }

    #[test]
    fn run_fails_when_workspace_is_not_initialized() {
        // Arrange
        let tmp = TempDir::new().unwrap();

        // Act
        let result = run(tmp.path(), "some-skill");

        // Assert
        assert!(result.is_err(), "should fail without skillet.toml");
    }

    #[test]
    fn load_config_uses_workspace_skills_dir_from_toml() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let custom_toml = "[workspace]\nskills_dir = 'custom-skills'\nfragments_dir = 'custom-skills/_fragments'\n\
            [lint]\nmax_activation_tokens = 4000\nmax_discovery_tokens = 100\nmax_fragment_tokens = 500\nallowed_commands = []\ndisable = []\n\
            [build]\ntokenizer = 'cl100k_base'\nverify_urls = false\n\
            [vars]\n[env]\n";
        fs::write(tmp.path().join("skillet.toml"), custom_toml).unwrap();

        // Act
        let config = crate::config::load(tmp.path()).unwrap();

        // Assert
        assert_eq!(config.workspace.skills_dir, "custom-skills");
    }
}
