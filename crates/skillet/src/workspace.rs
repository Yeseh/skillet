//! Skill and fragment discovery within a skillet workspace.

use anyhow::{Context, Result};
use sha2::Digest;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// A discovered skill source file within the workspace.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct SkillSource {
    /// The skill's directory name, used as its identifier.
    pub name: String,
    /// Absolute path to the `<name>.pan` source file.
    pub source_path: PathBuf,
    /// Absolute path to the skill's source directory (under `skills_src_dir`).
    pub skill_dir: PathBuf,
    /// Absolute path to the skill's output directory (under `skills_out_dir`).
    pub skill_out_dir: PathBuf,
}

/// Discovers all skill sources in `skills_src_dir`.
///
/// Scans one level deep, skipping directories whose names start with `_` or `.`.
/// The corresponding output directory for each skill is derived from `skills_out_dir`.
/// Returns entries sorted by name.
pub fn discover_skills(skills_src_dir: &Path, skills_out_dir: &Path) -> Result<Vec<SkillSource>> {
    let mut skills = Vec::new();

    if !skills_src_dir.exists() {
        return Ok(skills);
    }

    for entry in WalkDir::new(skills_src_dir)
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
            skills.push(SkillSource {
                name: dir_name.clone(),
                source_path,
                skill_dir: skill_dir.to_path_buf(),
                skill_out_dir: skills_out_dir.join(&dir_name),
            });
        }
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

/// Loads a fragment by name from `fragments_dir`.
///
/// Expects a file named `{name}.fragment.pan`.
pub fn load_fragment(fragments_dir: &Path, name: &str) -> Result<String> {
    let path = fragments_dir.join(format!("{}.fragment.pan", name));
    std::fs::read_to_string(&path).with_context(|| {
        format!(
            "fragment '{}' not found (expected at {})",
            name,
            path.display()
        )
    })
}

/// Returns `"sha256:<hex>"` of the file at `path`.
pub(crate) fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read {} for hashing", path.display()))?;
    Ok(format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(&bytes))
    ))
}

/// Returns `true` if `cmd` is found as a file in any directory on `PATH`.
pub(crate) fn is_on_path(cmd: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| dir.join(cmd).is_file())
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
        let src_dir = tmp.path().join("src/skills");
        let out_dir = tmp.path().join("skills");
        let dir = src_dir.join("diagnose");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("diagnose.pan"), "---\nname: diagnose\n---\n").unwrap();

        // Act
        let skills = discover_skills(&src_dir, &out_dir).unwrap();

        // Assert
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "diagnose");
        assert_eq!(skills[0].skill_out_dir, out_dir.join("diagnose"));
    }

    #[test]
    fn discover_skills_skips_underscore_dirs() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src/skills");
        let out_dir = tmp.path().join("skills");
        let frags = src_dir.join("_fragments");
        fs::create_dir_all(&frags).unwrap();
        fs::write(frags.join("_fragments.pan"), "").unwrap();

        // Act
        let skills = discover_skills(&src_dir, &out_dir).unwrap();

        // Assert
        assert!(skills.is_empty());
    }

    #[test]
    fn discover_skills_returns_empty_when_dir_missing() {
        // Arrange & Act
        let tmp = TempDir::new().unwrap();
        let out_dir = tmp.path().join("skills");
        let skills = discover_skills(&tmp.path().join("nonexistent"), &out_dir).unwrap();

        // Assert
        assert!(skills.is_empty());
    }

    #[test]
    fn load_fragment_reads_dot_fragment_pan_file() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("check-adrs.fragment.pan"),
            "## Check ADRs\n",
        )
        .unwrap();

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
