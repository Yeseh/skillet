//! Configuration types for `skillet.toml`.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// Global workspace settings. Owns only workspace-wide concerns.
/// Source and output directories live in `[module.*]` sections.
#[non_exhaustive]
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Path to workspace-global fragment files, shared across all modules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fragments_dir: Option<String>,
    /// Plugin marketplace publish settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish: Option<PublishConfig>,
}

/// Publish configuration controlling plugin marketplace output.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishConfig {
    /// Agent runtimes to emit marketplace manifests for.
    /// Supported values: `"claude"`, `"github-copilot"`.
    pub agents: Vec<String>,
    /// Marketplace identifier written into `marketplace.json`.
    pub marketplace_name: String,
    /// Marketplace owner name.
    pub owner_name: String,
    /// Marketplace owner contact email (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_email: Option<String>,
}

/// Per-module source/output configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleConfig {
    /// Path (relative to workspace root) where source `.pan` files live.
    pub src_dir: String,
    /// Path (relative to workspace root) where compiled outputs are written.
    pub out_dir: String,
    /// Published version of this module.
    pub version: String,
    /// Path to module-local fragment files (overrides global fragments of the same name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fragments_dir: Option<String>,
    /// Human-readable description written into `plugin.json` when publishing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether this module is included in the published marketplace.
    #[serde(default)]
    pub publish: bool,
}

/// Linting rules applied during `skillet lint`.
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub struct LintConfig {
    /// Maximum token budget for a skill's activation section.
    pub max_activation_tokens: u32,
    /// Maximum token budget for a skill's discovery section.
    pub max_discovery_tokens: u32,
    /// Maximum token budget for a single skill fragment.
    pub max_fragment_tokens: u32,
    /// Rule IDs to silence. Empty by default.
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
    /// Global workspace settings.
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    /// Named modules, each declaring a source/output pair.
    #[serde(default, rename = "module")]
    pub modules: BTreeMap<String, ModuleConfig>,
    /// Linting configuration (inherited by all modules).
    pub lint: LintConfig,
    /// Build configuration (inherited by all modules).
    pub build: BuildConfig,
    /// Freeform template variables available inside skill templates.
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    /// Environment variables with their default values.
    #[serde(default)]
    pub env: BTreeMap<String, EnvVar>,
    /// Commands treated as available regardless of PATH.
    #[serde(default)]
    pub allowed_commands: HashSet<String>,
}

impl Default for SkilletConfig {
    fn default() -> Self {
        let mut modules = BTreeMap::new();
        modules.insert(
            "default".to_string(),
            ModuleConfig {
                src_dir: "src/skills".to_string(),
                out_dir: "skills".to_string(),
                version: "0.1.0".to_string(),
                fragments_dir: Some("src/skills/_fragments".to_string()),
                description: None,
                publish: false,
            },
        );

        SkilletConfig {
            workspace: WorkspaceConfig::default(),
            modules,
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
            vars: BTreeMap::new(),
            env: BTreeMap::new(),
            allowed_commands: HashSet::default(),
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
    fn default_has_one_module_with_expected_dirs() {
        let cfg = SkilletConfig::default();
        assert_eq!(cfg.modules.len(), 1);
        let m = cfg.modules.get("default").expect("default module");
        assert_eq!(m.src_dir, "src/skills");
        assert_eq!(m.out_dir, "skills");
        assert_eq!(m.fragments_dir.as_deref(), Some("src/skills/_fragments"));
        assert_eq!(m.version, "0.1.0");
    }

    #[test]
    fn default_lint_token_limits_match_spec() {
        let cfg = SkilletConfig::default();
        assert_eq!(cfg.lint.max_activation_tokens, 4000);
        assert_eq!(cfg.lint.max_discovery_tokens, 100);
        assert_eq!(cfg.lint.max_fragment_tokens, 500);
    }

    #[test]
    fn default_build_uses_cl100k_base_tokenizer() {
        let cfg = SkilletConfig::default();
        assert_eq!(cfg.build.tokenizer, "cl100k_base");
        assert!(!cfg.build.verify_urls);
    }

    #[test]
    fn default_disable_list_is_empty() {
        let cfg = SkilletConfig::default();
        assert!(cfg.lint.disable.is_empty());
    }

    #[test]
    fn to_toml_round_trips_module_config() {
        let cfg = SkilletConfig::default();
        let toml_str = cfg.to_toml().unwrap();
        let parsed: toml::Value = toml::from_str(&toml_str).unwrap();

        assert_eq!(
            parsed["module"]["default"]["src_dir"].as_str().unwrap(),
            "src/skills"
        );
        assert_eq!(
            parsed["module"]["default"]["out_dir"].as_str().unwrap(),
            "skills"
        );
        assert_eq!(
            parsed["build"]["tokenizer"].as_str().unwrap(),
            "cl100k_base"
        );
    }

    #[test]
    fn parse_multi_module_toml() {
        let toml_str = r#"
[workspace]

[lint]
max_activation_tokens = 4000
max_discovery_tokens = 100
max_fragment_tokens = 500

[build]
tokenizer = "cl100k_base"

[module.core]
src_dir = "src/skills"
out_dir = "skills"
version = "1.0.0"

[module.plugin]
src_dir = "plugin/src"
out_dir = "plugin/out"
version = "0.2.0"
fragments_dir = "plugin/src/_fragments"
"#;
        let cfg: SkilletConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.modules.len(), 2);
        let core = cfg.modules.get("core").unwrap();
        assert_eq!(core.src_dir, "src/skills");
        assert_eq!(core.version, "1.0.0");
        assert!(core.fragments_dir.is_none());
        let plugin = cfg.modules.get("plugin").unwrap();
        assert_eq!(
            plugin.fragments_dir.as_deref(),
            Some("plugin/src/_fragments")
        );
    }
}
