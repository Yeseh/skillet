//! Configuration types for `skillet.toml`.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// Global workspace settings. Owns only workspace-wide concerns.
/// Source and output directories live in `[plugin.*]` sections.
#[non_exhaustive]
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Path to workspace-global fragment files, shared across all plugins.
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

/// An entry in a plugin's `dependencies` list, emitted into `dependencies.apm`
/// in the plugin's `apm.yml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ApmDependency {
    /// String shorthand, clone URL, or local path (APM schema §4.1.1).
    Shorthand(String),
    /// Remote Git or monorepo-sibling (`git: "parent"`) dependency object (§4.1.2).
    Git(GitApmDependency),
    /// Marketplace-resolved plugin dependency object (§4.1.2).
    Marketplace(MarketplaceApmDependency),
    /// Registry-resolved package dependency object (§4.1.2).
    Registry(RegistryApmDependency),
    /// Local path dependency object (§4.1.2).
    Local(LocalApmDependency),
}

/// Remote Git or monorepo-sibling (`git: "parent"`) APM dependency object.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitApmDependency {
    /// Git repository URL, shorthand, or `"parent"` for monorepo sibling.
    pub git: String,
    /// Optional subdirectory path inside the repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Optional Git ref (tag, branch, or commit SHA).
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// Optional local alias for this dependency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Optional repository type override.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub repo_type: Option<String>,
    /// Whether to allow insecure transport for this repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_insecure: Option<bool>,
    /// Optional subset of skills to install from this dependency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    /// Optional agent targets to scope this dependency to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<String>>,
}

/// Local filesystem path APM dependency object.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalApmDependency {
    /// Local path to the dependency package.
    pub path: String,
    /// Optional local alias for this dependency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Optional subset of skills to install from this dependency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    /// Optional agent targets to scope this dependency to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<String>>,
}

/// Marketplace-resolved plugin APM dependency object.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketplaceApmDependency {
    /// Plugin name in the marketplace.
    pub name: String,
    /// Marketplace identifier.
    pub marketplace: String,
    /// Optional version constraint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Registry-resolved package APM dependency object.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryApmDependency {
    /// Package identifier in the registry.
    pub id: String,
    /// Optional registry name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    /// Optional version constraint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Per-plugin source/output configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    /// Path (relative to workspace root) where source `.pan` files live.
    pub src_dir: String,
    /// Path (relative to workspace root) where compiled outputs are written.
    pub out_dir: String,
    /// Published version of this plugin.
    pub version: String,
    /// Path to plugin-local fragment files (overrides global fragments of the same name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fragments_dir: Option<String>,
    /// Human-readable description written into `plugin.json` when publishing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether this plugin is included in the published marketplace.
    #[serde(default)]
    pub publish: bool,
    /// Path (relative to workspace root) where `{name}.agent.pan` agent sources live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents_dir: Option<String>,
    /// External APM skill dependencies emitted into `dependencies.apm` in `apm.yml`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<ApmDependency>,
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
    /// Named plugins, each declaring a source/output pair.
    #[serde(default, rename = "plugin")]
    pub plugins: BTreeMap<String, PluginConfig>,
    /// Linting configuration (inherited by all plugins).
    pub lint: LintConfig,
    /// Build configuration (inherited by all plugins).
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
        let mut plugins = BTreeMap::new();
        plugins.insert(
            "default".to_string(),
            PluginConfig {
                src_dir: "src/skills".to_string(),
                out_dir: "skills".to_string(),
                version: "0.1.0".to_string(),
                fragments_dir: Some("src/skills/_fragments".to_string()),
                description: None,
                publish: false,
                agents_dir: None,
                dependencies: Vec::new(),
            },
        );

        SkilletConfig {
            workspace: WorkspaceConfig::default(),
            plugins,
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
    fn default_has_one_plugin_with_expected_dirs() {
        let cfg = SkilletConfig::default();
        assert_eq!(cfg.plugins.len(), 1);
        let m = cfg.plugins.get("default").expect("default plugin");
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
    fn to_toml_round_trips_plugin_config() {
        let cfg = SkilletConfig::default();
        let toml_str = cfg.to_toml().unwrap();
        let parsed: toml::Value = toml::from_str(&toml_str).unwrap();

        assert_eq!(
            parsed["plugin"]["default"]["src_dir"].as_str().unwrap(),
            "src/skills"
        );
        assert_eq!(
            parsed["plugin"]["default"]["out_dir"].as_str().unwrap(),
            "skills"
        );
        assert_eq!(
            parsed["build"]["tokenizer"].as_str().unwrap(),
            "cl100k_base"
        );
    }

    #[test]
    fn parse_multi_plugin_toml() {
        let toml_str = r#"
[workspace]

[lint]
max_activation_tokens = 4000
max_discovery_tokens = 100
max_fragment_tokens = 500

[build]
tokenizer = "cl100k_base"

[plugin.core]
src_dir = "src/skills"
out_dir = "skills"
version = "1.0.0"

[plugin.extras]
src_dir = "plugin/src"
out_dir = "plugin/out"
version = "0.2.0"
fragments_dir = "plugin/src/_fragments"
"#;
        let cfg: SkilletConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.plugins.len(), 2);
        let core = cfg.plugins.get("core").unwrap();
        assert_eq!(core.src_dir, "src/skills");
        assert_eq!(core.version, "1.0.0");
        assert!(core.fragments_dir.is_none());
        let extras = cfg.plugins.get("extras").unwrap();
        assert_eq!(
            extras.fragments_dir.as_deref(),
            Some("plugin/src/_fragments")
        );
        assert!(extras.dependencies.is_empty());
    }

    #[test]
    fn parse_plugin_dependencies_all_forms() {
        let toml_str = r#"
[workspace]

[lint]
max_activation_tokens = 4000
max_discovery_tokens = 100
max_fragment_tokens = 500

[build]
tokenizer = "cl100k_base"

[plugin.core]
src_dir = "src/skills"
out_dir = "skills"
version = "1.0.0"
dependencies = [
  "microsoft/apm-sample-package#v1.0.0",
  { git = "https://gitlab.com/acme/repo.git", path = "instructions/security", ref = "v2.0", alias = "acme-sec", type = "git", allow_insecure = false, skills = ["sec"], targets = ["claude"] },
  { path = "./packages/shared", alias = "shared", skills = ["lint"], targets = ["copilot"] },
  { name = "code-review", marketplace = "contoso", version = "^1.0.0" },
  { id = "acme/prompts", registry = "internal", version = "1.2.3" },
]
"#;
        let cfg: SkilletConfig = toml::from_str(toml_str).unwrap();
        let deps = &cfg.plugins["core"].dependencies;
        assert_eq!(deps.len(), 5);

        assert_eq!(
            deps[0],
            ApmDependency::Shorthand("microsoft/apm-sample-package#v1.0.0".to_string())
        );
        assert_eq!(
            deps[1],
            ApmDependency::Git(GitApmDependency {
                git: "https://gitlab.com/acme/repo.git".to_string(),
                path: Some("instructions/security".to_string()),
                git_ref: Some("v2.0".to_string()),
                alias: Some("acme-sec".to_string()),
                repo_type: Some("git".to_string()),
                allow_insecure: Some(false),
                skills: Some(vec!["sec".to_string()]),
                targets: Some(vec!["claude".to_string()]),
            })
        );
        assert_eq!(
            deps[2],
            ApmDependency::Local(LocalApmDependency {
                path: "./packages/shared".to_string(),
                alias: Some("shared".to_string()),
                skills: Some(vec!["lint".to_string()]),
                targets: Some(vec!["copilot".to_string()]),
            })
        );
        assert_eq!(
            deps[3],
            ApmDependency::Marketplace(MarketplaceApmDependency {
                name: "code-review".to_string(),
                marketplace: "contoso".to_string(),
                version: Some("^1.0.0".to_string()),
            })
        );
        assert_eq!(
            deps[4],
            ApmDependency::Registry(RegistryApmDependency {
                id: "acme/prompts".to_string(),
                registry: Some("internal".to_string()),
                version: Some("1.2.3".to_string()),
            })
        );
    }

    #[test]
    fn parse_plugin_dependencies_rejects_unknown_fields() {
        let toml_str = r#"
[workspace]

[lint]
max_activation_tokens = 4000
max_discovery_tokens = 100
max_fragment_tokens = 500

[build]
tokenizer = "cl100k_base"

[plugin.core]
src_dir = "src/skills"
out_dir = "skills"
version = "1.0.0"
dependencies = [
  { git = "owner/repo", bogus_field = true },
]
"#;
        let err = toml::from_str::<SkilletConfig>(toml_str).unwrap_err();
        assert!(
            err.to_string().contains("dependencies"),
            "unexpected error: {err}"
        );
    }
}
