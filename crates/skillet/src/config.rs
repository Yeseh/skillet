//! Configuration types for `skillet.toml`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Directory layout for the skillet workspace.
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Path (relative to the project root) where skill source `.pan` files are stored.
    pub skills_src_dir: String,
    /// Path (relative to the project root) where compiled `SKILL.md` outputs are written.
    pub skills_out_dir: String,
    /// Path where skill fragment `.fragment.pan` files are stored.
    pub fragments_dir: String,
}

/// Linting rules applied during `skillet check`.
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub struct LintConfig {
    /// Maximum token budget for a skill's activation section.
    pub max_activation_tokens: u32,
    /// Maximum token budget for a skill's discovery section.
    pub max_discovery_tokens: u32,
    /// Maximum token budget for a single skill fragment.
    pub max_fragment_tokens: u32,
    /// Shell commands that skills are permitted to invoke (e.g. `"docker"`, `"kubectl"`).
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    /// Rule IDs to silence (e.g. `"lint-missing-docs"`).  Empty by default.
    #[serde(default)]
    pub disable: Vec<String>,
}

/// Build settings controlling how skills are compiled.
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Tokenizer model used for token counting (e.g. `"cl100k_base"`).
    pub tokenizer: String,
    /// Whether to verify that URLs referenced in skills are reachable.
    #[serde(default)]
    pub verify_urls: bool,
}

/// A template variable backed by an environment variable.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVar {
    /// Value used when the environment variable is not set.
    pub default: String,
}

/// Top-level skillet configuration written to `skillet.toml`.
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub struct SkilletConfig {
    /// Workspace directory settings.
    pub workspace: WorkspaceConfig,
    /// Linting configuration.
    pub lint: LintConfig,
    /// Build configuration.
    pub build: BuildConfig,
    /// Freeform template variables available inside skill templates.
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    /// Environment variables with their default values.
    #[serde(default)]
    pub env: BTreeMap<String, EnvVar>,
}

impl Default for SkilletConfig {
    fn default() -> Self {
        let mut vars = BTreeMap::new();
        vars.insert("project_name".to_string(), "my-project".to_string());

        let mut env = BTreeMap::new();
        env.insert(
            "CI".to_string(),
            EnvVar {
                default: "false".to_string(),
            },
        );
        env.insert(
            "TEAM_NAME".to_string(),
            EnvVar {
                default: "engineering".to_string(),
            },
        );

        SkilletConfig {
            workspace: WorkspaceConfig {
                skills_src_dir: "src/skills".to_string(),
                skills_out_dir: "skills".to_string(),
                fragments_dir: "src/skills/_fragments".to_string(),
            },
            lint: LintConfig {
                max_activation_tokens: 4000,
                max_discovery_tokens: 100,
                max_fragment_tokens: 500,
                allowed_commands: vec![],
                disable: vec![],
            },
            build: BuildConfig {
                tokenizer: "cl100k_base".to_string(),
                verify_urls: false,
            },
            vars,
            env,
        }
    }
}

impl SkilletConfig {
    /// Serializes this configuration to a pretty-printed TOML string.
    pub fn to_toml(&self) -> anyhow::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_workspace_dirs_are_nested_under_skills() {
        // Arrange & Act
        let cfg = SkilletConfig::default();

        // Assert
        assert_eq!(cfg.workspace.skills_src_dir, "src/skills");
        assert_eq!(cfg.workspace.skills_out_dir, "skills");
        assert_eq!(cfg.workspace.fragments_dir, "src/skills/_fragments");
    }

    #[test]
    fn default_lint_token_limits_match_spec() {
        // Arrange & Act
        let cfg = SkilletConfig::default();

        // Assert
        assert_eq!(cfg.lint.max_activation_tokens, 4000);
        assert_eq!(cfg.lint.max_discovery_tokens, 100);
        assert_eq!(cfg.lint.max_fragment_tokens, 500);
    }

    #[test]
    fn default_build_uses_cl100k_base_tokenizer() {
        // Arrange & Act
        let cfg = SkilletConfig::default();

        // Assert
        assert_eq!(cfg.build.tokenizer, "cl100k_base");
        assert!(!cfg.build.verify_urls);
    }

    #[test]
    fn default_disable_list_is_empty() {
        // Arrange & Act
        let cfg = SkilletConfig::default();

        // Assert
        assert!(cfg.lint.disable.is_empty());
    }

    #[test]
    fn to_toml_round_trips_expected_values() {
        // Arrange
        let cfg = SkilletConfig::default();

        // Act
        let toml_str = cfg.to_toml().unwrap();
        let parsed: toml::Value = toml::from_str(&toml_str).unwrap();

        // Assert
        assert_eq!(
            parsed["workspace"]["skills_src_dir"].as_str().unwrap(),
            "src/skills"
        );
        assert_eq!(
            parsed["workspace"]["skills_out_dir"].as_str().unwrap(),
            "skills"
        );
        assert_eq!(
            parsed["build"]["tokenizer"].as_str().unwrap(),
            "cl100k_base"
        );
        assert_eq!(
            parsed["vars"]["project_name"].as_str().unwrap(),
            "my-project"
        );
        assert_eq!(parsed["env"]["CI"]["default"].as_str().unwrap(), "false");
    }
}
