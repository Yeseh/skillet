//! Parsed representations of `.pan` source files.
//!
//! Centralises frontmatter extraction so every caller — build, lint, and
//! budget — works from the same type rather than defining its own local
//! `Deserialize` struct.

use gray_matter::{engine::YAML, Matter};
use serde::Deserialize;

/// The YAML frontmatter of a `.pan` skill source file.
///
/// All fields are `Option` so callers can report missing-field errors rather
/// than receiving a parse failure from the deserializer.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillFrontmatter {
    /// Skill identifier — must match the containing directory name.
    pub name: Option<String>,
    /// Short description shown in discovery-token reporting.
    pub description: Option<String>,
}

/// Parses the YAML frontmatter of a `.pan` source string.
///
/// Returns `None` when no frontmatter delimiters (`---`) are present.
///
/// # Errors
///
/// Returns an error if the frontmatter YAML cannot be deserialized.
pub fn parse_frontmatter(source: &str) -> anyhow::Result<Option<SkillFrontmatter>> {
    let matter = Matter::<YAML>::new();
    let parsed = matter
        .parse::<SkillFrontmatter>(source.strip_prefix('\u{feff}').unwrap_or(source))
        .map_err(|e| anyhow::anyhow!("failed to parse frontmatter: {e}"))?;
    Ok(parsed.data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_returns_name_and_description() {
        // Arrange
        let src = "---\nname: diagnose\ndescription: A hard-bug loop.\n---\n\n# body\n";

        // Act
        let fm = parse_frontmatter(src).unwrap().unwrap();

        // Assert
        assert_eq!(fm.name.as_deref(), Some("diagnose"));
        assert_eq!(fm.description.as_deref(), Some("A hard-bug loop."));
    }

    #[test]
    fn parse_frontmatter_returns_none_when_no_delimiters() {
        // Arrange
        let src = "# No frontmatter\n";

        // Act
        let result = parse_frontmatter(src).unwrap();

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn parse_frontmatter_fields_are_optional() {
        // Arrange — only name, no description
        let src = "---\nname: foo\n---\n\n# body\n";

        // Act
        let fm = parse_frontmatter(src).unwrap().unwrap();

        // Assert
        assert_eq!(fm.name.as_deref(), Some("foo"));
        assert!(fm.description.is_none());
    }
}
