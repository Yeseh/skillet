//! Skill and fragment discovery within a skillet workspace.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// A discovered skill source file within the workspace.
#[non_exhaustive]
#[derive(Debug)]
pub struct SkillSource {
    /// The skill's directory name, used as its identifier.
    pub name: String,
    /// Absolute path to the `<name>.skill` source file.
    pub source_path: PathBuf,
    /// Absolute path to the skill's directory.
    pub skill_dir: PathBuf,
}

/// Discovers all skill sources in `skills_dir`.
///
/// Scans one level deep, skipping directories whose names start with `_` or `.`.
/// Returns entries sorted by name.
pub fn discover_skills(skills_dir: &Path) -> Result<Vec<SkillSource>> {
    let mut skills = Vec::new();

    if !skills_dir.exists() {
        return Ok(skills);
    }

    for entry in WalkDir::new(skills_dir)
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
        let source_path = skill_dir.join(format!("{}.skill", dir_name));
        if source_path.exists() {
            skills.push(SkillSource {
                name: dir_name,
                source_path,
                skill_dir: skill_dir.to_path_buf(),
            });
        }
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

/// Loads a fragment by name from `fragments_dir`.
///
/// Expects a file named `{name}.fragment.skill`.
pub fn load_fragment(fragments_dir: &Path, name: &str) -> Result<String> {
    let path = fragments_dir.join(format!("{}.fragment.skill", name));
    std::fs::read_to_string(&path).with_context(|| {
        format!(
            "fragment '{}' not found (expected at {})",
            name,
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn discover_skills_finds_skill_source_in_subdir() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("diagnose");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("diagnose.skill"), "---\nname: diagnose\n---\n").unwrap();

        // Act
        let skills = discover_skills(tmp.path()).unwrap();

        // Assert
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "diagnose");
    }

    #[test]
    fn discover_skills_skips_underscore_dirs() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let frags = tmp.path().join("_fragments");
        fs::create_dir_all(&frags).unwrap();
        fs::write(frags.join("_fragments.skill"), "").unwrap();

        // Act
        let skills = discover_skills(tmp.path()).unwrap();

        // Assert
        assert!(skills.is_empty());
    }

    #[test]
    fn discover_skills_returns_empty_when_dir_missing() {
        // Arrange & Act
        let tmp = TempDir::new().unwrap();
        let skills = discover_skills(&tmp.path().join("nonexistent")).unwrap();

        // Assert
        assert!(skills.is_empty());
    }

    #[test]
    fn load_fragment_reads_dot_fragment_skill_file() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("check-adrs.fragment.skill"), "## Check ADRs\n").unwrap();

        // Act
        let content = load_fragment(tmp.path(), "check-adrs").unwrap();

        // Assert
        assert_eq!(content, "## Check ADRs\n");
    }

    #[test]
    fn load_fragment_errors_when_file_missing() {
        // Arrange
        let tmp = TempDir::new().unwrap();

        // Act
        let result = load_fragment(tmp.path(), "missing");

        // Assert
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing"));
    }
}
