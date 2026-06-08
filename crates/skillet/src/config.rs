//! Configuration types for `skillet.toml`.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// Directory layout for the skillet workspace.
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Path (relative to the project root) where source `.pan` files are stored.
    pub src_dir: String,
    /// Path (relative to the project root) where compiled outputs are written.
    pub out_dir: String,
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
    /// The allowed commands
    #[serde(default)]
    pub allowed_commands: HashSet<String>
}

impl Default for SkilletConfig {
    fn default() -> Self {
        let vars = BTreeMap::new();
        let env = BTreeMap::new();

        SkilletConfig {
            workspace: WorkspaceConfig {
                src_dir: "src/skills".to_string(),
                out_dir: "skills".to_string(),
                fragments_dir: "src/skills/_fragments".to_string(),
            },
            lint: LintConfig {
                max_activation_tokens: 4000,
                max_discovery_tokens: 100,
                max_fragment_tokens: 500,
                disable: vec![],
            },
            build: BuildConfig {
                tokenizer: "cl100k_base".to_string(),
                verify_urls: false,
            },
            vars,
            env,
            allowed_commands: HashSet::default()
        }
    }
}

impl SkilletConfig {
    /// Serializes this configuration to a pretty-printed TOML string.
    pub fn to_toml(&self) -> anyhow::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Loads `skillet.toml` from the given workspace root.
    pub fn load(workspace: &Path) -> anyhow::Result<Self> {
        let toml_path = workspace.join("skillet.toml");
        let raw = std::fs::read_to_string(&toml_path).with_context(|| {
            format!(
                "cannot read {}: workspace not initialized (run `skillet init` first)",
                toml_path.display()
            )
        })?;
        toml::from_str(&raw).context("failed to parse skillet.toml")
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
        assert_eq!(cfg.workspace.src_dir, "src/skills");
        assert_eq!(cfg.workspace.out_dir, "skills");
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
            parsed["workspace"]["src_dir"].as_str().unwrap(),
            "src/skills"
        );
        assert_eq!(
            parsed["workspace"]["out_dir"].as_str().unwrap(),
            "skills"
        );
        assert_eq!(
            parsed["build"]["tokenizer"].as_str().unwrap(),
            "cl100k_base"
        );
        // vars and env are empty by default; just verify the sections exist as tables
        assert!(parsed.get("vars").is_some());
        assert!(parsed.get("env").is_some());
    }
}
